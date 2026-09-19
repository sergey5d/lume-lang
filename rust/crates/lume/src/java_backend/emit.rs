use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
};

use crate::{
    ast::{self, TypeKind, TypeRef, Visibility},
    backend::BackendBundle,
    core,
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
    push_class_equality_bridge(&mut out, bundle, ty, names);
    push_class_hash_bridge(&mut out, bundle, ty);
    out.push_str("}\n");
    out
}

fn push_class_hash_bridge(out: &mut String, bundle: &BackendBundle, ty: &ir::TypeDef) {
    if !java_type_has_bound(bundle, ty, "Hashed", &mut HashSet::new()) {
        return;
    }
    let has_hash_method = ty.methods.iter().any(|method| {
        bundle
            .ir
            .function(*method)
            .is_some_and(|function| function.name == "hash" && function.params.is_empty())
    });
    if !has_hash_method {
        return;
    }

    out.push_str("\n    @Override\n");
    out.push_str("    public int hashCode() {\n");
    out.push_str("        return Long.hashCode(this.hash());\n");
    out.push_str("    }\n");
}

fn push_class_equality_bridge(
    out: &mut String,
    bundle: &BackendBundle,
    ty: &ir::TypeDef,
    names: &JavaNames,
) {
    let equality_param = ty.methods.iter().find_map(|method| {
        let Some(function) = bundle.ir.function(*method) else {
            return None;
        };
        if function.name != "equals" || function.params.len() != 1 {
            return None;
        }
        let Some(param) = function
            .params
            .first()
            .and_then(|param| function.locals.get(param.0))
        else {
            return None;
        };
        match &param.ty {
            ir::Type::Named { name, .. } => (name == &ty.name
                || java_type_has_bound(bundle, ty, name, &mut HashSet::new()))
            .then_some(param.ty.clone()),
            _ => None,
        }
    });
    let Some(ir::Type::Named {
        name: domain_name,
        args: domain_args,
    }) = equality_param
    else {
        return;
    };

    let domain_name = names.named_type(&domain_name);
    let pattern_type = if domain_args.is_empty() {
        domain_name.clone()
    } else {
        format!(
            "{}<{}>",
            domain_name,
            std::iter::repeat_n("?", domain_args.len())
                .collect::<Vec<_>>()
                .join(", ")
        )
    };
    let argument = if domain_args.is_empty() {
        "that".to_string()
    } else {
        format!("({domain_name}) that")
    };

    out.push_str("\n    @Override\n");
    out.push_str("    public boolean equals(Object other) {\n");
    out.push_str("        if (this == other) return true;\n");
    out.push_str(&format!(
        "        if (!(other instanceof {pattern_type} that)) return false;\n"
    ));
    out.push_str(&format!(
        "        return Boolean.TRUE.equals(this.equals({argument}));\n"
    ));
    out.push_str("    }\n");
}

fn java_type_has_bound(
    bundle: &BackendBundle,
    ty: &ir::TypeDef,
    expected: &str,
    seen: &mut HashSet<ir::TypeId>,
) -> bool {
    if !seen.insert(ty.id) {
        return false;
    }
    ty.with_bounds.iter().any(|bound| {
        let ir::Type::Named { name, .. } = bound else {
            return false;
        };
        name == expected
            || bundle
                .ir
                .types
                .iter()
                .find(|candidate| candidate.name == *name)
                .is_some_and(|parent| java_type_has_bound(bundle, parent, expected, seen))
    })
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
    push_shape_value_methods(&mut out, ty);
    push_instance_methods(&mut out, bundle, ty, MethodShell::StubBody, names);
    out.push_str("}\n");
    out
}

fn push_shape_value_methods(out: &mut String, ty: &ir::TypeDef) {
    let shape_name = java_type_name(&ty.name);
    let pattern_type = if ty.type_params.is_empty() {
        shape_name.clone()
    } else {
        format!(
            "{}<{}>",
            shape_name,
            std::iter::repeat_n("?", ty.type_params.len())
                .collect::<Vec<_>>()
                .join(", ")
        )
    };

    out.push_str("\n    @Override\n");
    out.push_str("    public boolean equals(Object other) {\n");
    out.push_str("        if (this == other) return true;\n");
    out.push_str(&format!(
        "        if (!(other instanceof {pattern_type} that)) return false;\n"
    ));
    if ty.fields.is_empty() {
        out.push_str("        return true;\n");
    } else {
        out.push_str("        return ");
        for (index, field) in ty.fields.iter().enumerate() {
            if index > 0 {
                out.push_str("\n            && ");
            }
            let field_name = java_member_name(&field.name);
            out.push_str(&format!(
                "java.util.Objects.equals(this.{field_name}, that.{field_name})"
            ));
        }
        out.push_str(";\n");
    }
    out.push_str("    }\n");

    out.push_str("\n    @Override\n");
    out.push_str("    public int hashCode() {\n");
    out.push_str("        return java.util.Objects.hash(");
    let mut hash_fields = ty.fields.iter().collect::<Vec<_>>();
    hash_fields.sort_by(|left, right| left.name.cmp(&right.name));
    out.push_str(
        &hash_fields
            .iter()
            .map(|field| format!("this.{}", java_member_name(&field.name)))
            .collect::<Vec<_>>()
            .join(", "),
    );
    out.push_str(");\n");
    out.push_str("    }\n");
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
        push_union_variant(
            &mut out,
            case,
            &enum_name,
            &type_params,
            &type_args,
            &ty.type_params,
            names,
        );
    }

    out.push_str("}\n");
    out
}

fn push_union_variant(
    out: &mut String,
    case: &ir::EnumCase,
    union_name: &str,
    type_params: &str,
    type_args: &str,
    type_param_names: &[String],
    names: &JavaNames,
) {
    let case_name = java_type_name(&case.name);
    match case.kind {
        TypeKind::Object => {
            out.push_str(&format!(
                "    final class {case_name}{type_params} implements {union_name}{type_args} {{\n"
            ));
            let wildcards = java_wildcard_type_args(type_param_names.len());
            let constructor_args = if type_param_names.is_empty() {
                ""
            } else {
                "<>"
            };
            out.push_str(&format!(
                "        private static final {case_name}{wildcards} INSTANCE = new {case_name}{constructor_args}();\n"
            ));
            out.push_str(&format!("        private {case_name}() {{}}\n"));
            if type_param_names.is_empty() {
                out.push_str(&format!(
                    "        public static {case_name} instance() {{ return INSTANCE; }}\n"
                ));
            } else {
                out.push_str("        @SuppressWarnings(\"unchecked\")\n");
                out.push_str(&format!(
                    "        public static {type_params} {case_name}{type_args} instance() {{ return ({case_name}{type_args}) INSTANCE; }}\n"
                ));
            }
            out.push_str(&format!(
                "        @Override public String toString() {{ return \"{case_name}\"; }}\n"
            ));
            out.push_str("    }\n");
        }
        TypeKind::Class => {
            out.push_str(&format!(
                "    final class {case_name}{type_params} implements {union_name}{type_args} {{\n"
            ));
            for field in &case.fields {
                let final_modifier = if field.mutable { "" } else { "final " };
                out.push_str(&format!(
                    "        {final_modifier}{} {};\n",
                    names.value_type(&field.ty),
                    java_member_name(&field.name)
                ));
            }
            out.push_str(&format!("        public {case_name}("));
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
            out.push_str(") {\n");
            for field in &case.fields {
                let name = java_member_name(&field.name);
                out.push_str(&format!("            this.{name} = {name};\n"));
            }
            out.push_str("        }\n");
            for field in &case.fields {
                let name = java_member_name(&field.name);
                out.push_str(&format!(
                    "        public {} {name}() {{ return {name}; }}\n",
                    names.value_type(&field.ty)
                ));
            }
            out.push_str("    }\n");
        }
        TypeKind::Record | TypeKind::Enum => {
            out.push_str("    record ");
            out.push_str(&case_name);
            out.push_str(type_params);
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
            out.push_str(union_name);
            out.push_str(type_args);
            out.push_str(" {}\n");
        }
        _ => unreachable!("unsupported declared union variant kind"),
    }
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
    let mut bounds = ty
        .with_bounds
        .iter()
        .filter_map(|bound| java_bound_type(bound, names))
        .collect::<Vec<_>>();
    if ty.kind != TypeKind::Annotation && !bounds.iter().any(|bound| bound == "lume.core.LumeTyped")
    {
        bounds.push("lume.core.LumeTyped".to_string());
    }
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
        ir::Type::TypeParam(_) | ir::Type::Unknown | ir::Type::Union(_) => "Object".to_string(),
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
        ir::Type::Union(members) => format!(
            "lume.core.LumeType.primitive({})",
            java_string_literal(
                &members
                    .iter()
                    .map(type_descriptor_name)
                    .collect::<Vec<_>>()
                    .join(" | ")
            )
        ),
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
        ir::Type::Union(members) => members
            .iter()
            .map(type_descriptor_name)
            .collect::<Vec<_>>()
            .join(" | "),
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

#[derive(Debug, Clone)]
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

fn source_function_param_specs(function: &ir::Function) -> Vec<JavaParamSpec> {
    function
        .params
        .iter()
        .enumerate()
        .filter_map(|(index, param)| {
            let local = function.locals.get(param.0)?;
            (local.name != "this" && !is_reified_type_param_local(&local.name)).then(|| {
                JavaParamSpec {
                    ty: local.ty.clone(),
                    variadic: function.param_variadic.get(index).copied().unwrap_or(false),
                    lazy: function.param_lazy.get(index).copied().unwrap_or(false),
                    default: function.param_defaults.get(index).cloned().flatten(),
                    coercion: None,
                }
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
    match structured_source_function_body(bundle, function, names) {
        Some(body) => out.push_str(&body),
        None => push_stub_body(out),
    }
}

fn structured_source_function_body(
    bundle: &BackendBundle,
    function: &ir::Function,
    names: &JavaNames,
) -> Option<String> {
    let body = bundle.core_bodies.get(&function.id)?;
    let owner = match function.kind {
        FunctionKind::Method { owner } => bundle.ir.types.get(owner.0),
        FunctionKind::TopLevel => None,
        FunctionKind::Local { .. } | FunctionKind::Lambda | FunctionKind::Synthetic => {
            return None;
        }
    };
    SourceBodyEmitter {
        bundle,
        function,
        names,
        owner,
    }
    .emit_body(body)
}

fn structured_source_function_body_with_local_overrides(
    bundle: &BackendBundle,
    function: &ir::Function,
    names: &JavaNames,
    local_overrides: &HashMap<ir::LocalId, String>,
) -> Option<String> {
    let body = bundle.core_bodies.get(&function.id)?;
    let owner = match function.kind {
        FunctionKind::Method { owner } => bundle.ir.types.get(owner.0),
        FunctionKind::TopLevel | FunctionKind::Local { .. } | FunctionKind::Lambda => None,
        FunctionKind::Synthetic => return None,
    };
    let mut bindings = HashMap::new();
    let mut binding_types = HashMap::new();
    for (local_id, java_name) in local_overrides {
        let local = function.locals.get(local_id.0)?;
        bindings.insert(local.name.clone(), java_name.clone());
        binding_types.insert(local.name.clone(), local.ty.clone());
    }
    SourceBodyEmitter {
        bundle,
        function,
        names,
        owner,
    }
    .emit_body_with_bindings(body, bindings, binding_types)
}

fn structured_source_constructor_body(
    bundle: &BackendBundle,
    function: &ir::Function,
    names: &JavaNames,
    prologue: Option<&str>,
) -> Option<String> {
    let body = bundle.core_bodies.get(&function.id)?;
    let FunctionKind::Method { owner } = function.kind else {
        return None;
    };
    let owner = bundle.ir.types.get(owner.0)?;
    let mut emitted = SourceBodyEmitter {
        bundle,
        function,
        names,
        owner: Some(owner),
    }
    .emit_body(body)?;
    if let Some(prologue) = prologue {
        emitted.insert_str(3, &format!("        {prologue}\n"));
    }
    Some(emitted)
}

struct SourceBodyEmitter<'a> {
    bundle: &'a BackendBundle,
    function: &'a ir::Function,
    names: &'a JavaNames,
    owner: Option<&'a ir::TypeDef>,
}

impl<'a> SourceBodyEmitter<'a> {
    fn emit_body(&self, body: &core::CallableBody) -> Option<String> {
        self.emit_body_with_bindings(body, HashMap::new(), HashMap::new())
    }

    fn emit_body_with_bindings(
        &self,
        body: &core::CallableBody,
        mut bindings: HashMap<String, String>,
        mut binding_types: HashMap<String, ir::Type>,
    ) -> Option<String> {
        let mut out = String::new();
        out.push_str(" {\n");
        match body {
            core::CallableBody::Expr(expr) => {
                self.emit_returning_expr(&mut out, expr, "        ", &bindings, &binding_types)?
            }
            core::CallableBody::Block(block) => self.emit_statement_block(
                &mut out,
                block,
                "        ",
                &mut bindings,
                &mut binding_types,
                &mut HashSet::new(),
                true,
                0,
            )?,
        }
        out.push_str("    }\n");
        Some(out)
    }

    #[allow(clippy::too_many_arguments)]
    fn emit_statement_block(
        &self,
        out: &mut String,
        block: &core::Block,
        indent: &str,
        bindings: &mut HashMap<String, String>,
        binding_types: &mut HashMap<String, ir::Type>,
        used_locals: &mut HashSet<ir::LocalId>,
        returns_tail: bool,
        loop_depth: usize,
    ) -> Option<()> {
        for (index, statement) in block.statements.iter().enumerate() {
            let is_tail = index + 1 == block.statements.len();
            match statement {
                core::Stmt::Binding(binding)
                    if binding.destructure.is_none()
                        && binding.bindings.len() == 1
                        && matches!(binding.values.as_slice(), [core::Expr::Try { .. }]) =>
                {
                    let core::Expr::Try { value, span } = &binding.values[0] else {
                        unreachable!();
                    };
                    self.emit_try_binding(
                        out,
                        &binding.bindings[0],
                        value,
                        *span,
                        indent,
                        bindings,
                        binding_types,
                        used_locals,
                    )?;
                }
                core::Stmt::Binding(binding) if binding.destructure.is_some() => {
                    if matches!(binding.values.as_slice(), [core::Expr::If { .. }]) {
                        self.emit_if_destructuring_binding(
                            out,
                            binding,
                            indent,
                            bindings,
                            binding_types,
                            used_locals,
                        )?;
                    } else {
                        self.emit_flat_destructuring_binding(
                            out,
                            binding.destructure?,
                            &binding.bindings,
                            binding.values.as_slice(),
                            binding.span,
                            indent,
                            bindings,
                            binding_types,
                            used_locals,
                        )?;
                    }
                }
                core::Stmt::Binding(binding)
                    if binding.destructure.is_none()
                        && binding.bindings.len() == 1
                        && matches!(binding.values.as_slice(), [core::Expr::If { .. }]) =>
                {
                    let [value] = binding.values.as_slice() else {
                        unreachable!();
                    };
                    self.emit_if_value_binding(
                        out,
                        &binding.bindings[0],
                        value,
                        indent,
                        bindings,
                        binding_types,
                        used_locals,
                    )?;
                }
                core::Stmt::Binding(binding)
                    if binding.destructure.is_none()
                        && binding.bindings.len() == binding.values.len() =>
                {
                    for (binding, value) in binding.bindings.iter().zip(&binding.values) {
                        if binding.name == "_" {
                            out.push_str(indent);
                            out.push_str(&self.emit_expr(value, &bindings)?);
                            out.push_str(";\n");
                            continue;
                        }
                        let local = self.source_binding_local(&binding.name, &used_locals)?;
                        used_locals.insert(local.id);
                        let java_name = java_local_name(local);
                        if let Some(Some(call)) =
                            self.emit_call_with_hoisted_try_args(out, value, indent, bindings)
                        {
                            out.push_str(indent);
                            out.push_str(&self.names.value_type(&local.ty));
                            out.push(' ');
                            out.push_str(&java_name);
                            out.push_str(" = ");
                            out.push_str(&call);
                            out.push_str(";\n");
                            bindings.insert(binding.name.clone(), java_name);
                            binding_types.insert(binding.name.clone(), local.ty.clone());
                            continue;
                        }
                        if self.emit_short_circuit_try_binding(
                            out, value, &java_name, &local.ty, indent, bindings,
                        )? {
                            bindings.insert(binding.name.clone(), java_name);
                            binding_types.insert(binding.name.clone(), local.ty.clone());
                            continue;
                        }
                        out.push_str(indent);
                        out.push_str(&self.names.value_type(&local.ty));
                        out.push(' ');
                        out.push_str(&java_name);
                        out.push_str(" = ");
                        out.push_str(&self.emit_expr_against(value, &bindings, &local.ty)?);
                        out.push_str(";\n");
                        bindings.insert(binding.name.clone(), java_name);
                        binding_types.insert(binding.name.clone(), local.ty.clone());
                    }
                }
                core::Stmt::Return(statement) => {
                    out.push_str(indent);
                    out.push_str("return");
                    if let Some(value) = &statement.value {
                        out.push(' ');
                        out.push_str(&self.emit_expr_against(
                            value,
                            &bindings,
                            &self.function.return_ty,
                        )?);
                    }
                    out.push_str(";\n");
                }
                core::Stmt::Assignment(statement)
                    if statement.targets.len() == 1
                        && statement.values.len() == 1
                        && matches!(
                            statement.operator,
                            ast::AssignOp::Assign | ast::AssignOp::Reassign
                        )
                        && matches!(statement.targets.as_slice(), [core::Expr::Index { .. }]) =>
                {
                    let core::Expr::Index {
                        receiver, index, ..
                    } = &statement.targets[0]
                    else {
                        unreachable!();
                    };
                    let ir::Type::Named { name, args } = self.expr_type(receiver, bindings)? else {
                        return None;
                    };
                    if name != "Map" || args.len() != 2 {
                        return None;
                    }
                    out.push_str(indent);
                    out.push_str(&self.emit_expr(receiver, bindings)?);
                    out.push_str(".set(");
                    out.push_str(&self.emit_expr_against(index, bindings, &args[0])?);
                    out.push_str(", ");
                    out.push_str(&self.emit_expr_against(
                        &statement.values[0],
                        bindings,
                        &args[1],
                    )?);
                    out.push_str(");\n");
                }
                core::Stmt::Assignment(statement)
                    if statement.targets.len() == 1
                        && statement.values.len() == 1
                        && matches!(statement.values.as_slice(), [core::Expr::Try { .. }])
                        && matches!(
                            statement.operator,
                            ast::AssignOp::Assign | ast::AssignOp::Reassign
                        ) =>
                {
                    let core::Expr::Try { value, span } = &statement.values[0] else {
                        unreachable!();
                    };
                    let target = self.emit_assignment_target(&statement.targets[0], bindings)?;
                    let target_ty = self.assignment_target_type(
                        &statement.targets[0],
                        bindings,
                        binding_types,
                    )?;
                    self.emit_try_assignment(
                        out, &target, &target_ty, value, *span, indent, bindings,
                    )?;
                }
                core::Stmt::Assignment(statement)
                    if statement.targets.len() == 1 && statement.values.len() == 1 =>
                {
                    let target = self.emit_assignment_target(&statement.targets[0], &bindings)?;
                    let value = match statement.operator {
                        ast::AssignOp::Assign | ast::AssignOp::Reassign => self
                            .assignment_target_type(
                                &statement.targets[0],
                                &bindings,
                                &binding_types,
                            )
                            .and_then(|target_ty| {
                                self.emit_expr_against(&statement.values[0], &bindings, &target_ty)
                            })
                            .or_else(|| self.emit_expr(&statement.values[0], &bindings))?,
                        _ => self.emit_expr(&statement.values[0], &bindings)?,
                    };
                    let operator = match statement.operator {
                        ast::AssignOp::Assign | ast::AssignOp::Reassign => "=",
                        ast::AssignOp::AddAssign => "+=",
                        ast::AssignOp::SubAssign => "-=",
                        ast::AssignOp::MulAssign => "*=",
                        ast::AssignOp::DivAssign => "/=",
                        ast::AssignOp::ModAssign => "%=",
                    };
                    out.push_str(indent);
                    out.push_str(&target);
                    out.push(' ');
                    out.push_str(operator);
                    out.push(' ');
                    out.push_str(&value);
                    out.push_str(";\n");
                }
                core::Stmt::LetElse(statement) => self.emit_let_else_statement(
                    out,
                    statement,
                    indent,
                    bindings,
                    binding_types,
                    used_locals,
                    loop_depth,
                )?,
                core::Stmt::If(statement) => {
                    let branches_return =
                        is_tail && returns_tail && !is_java_void_type(&self.function.return_ty);
                    self.emit_if_statement(
                        out,
                        statement,
                        indent,
                        bindings,
                        binding_types,
                        used_locals,
                        loop_depth,
                        branches_return,
                    )?;
                }
                core::Stmt::Match(statement) => {
                    let branches_return =
                        is_tail && returns_tail && !is_java_void_type(&self.function.return_ty);
                    self.emit_match_statement(
                        out,
                        statement,
                        indent,
                        bindings,
                        binding_types,
                        used_locals,
                        loop_depth,
                        branches_return,
                    )?;
                }
                core::Stmt::While(statement) => self.emit_while_statement(
                    out,
                    statement,
                    indent,
                    bindings,
                    binding_types,
                    used_locals,
                    loop_depth,
                )?,
                core::Stmt::For(statement) => self.emit_for_statement(
                    out,
                    statement,
                    indent,
                    bindings,
                    binding_types,
                    used_locals,
                    loop_depth,
                )?,
                core::Stmt::Break(_) if loop_depth > 0 => {
                    out.push_str(indent);
                    out.push_str("break;\n");
                }
                core::Stmt::Continue(_) if loop_depth > 0 => {
                    out.push_str(indent);
                    out.push_str("continue;\n");
                }
                core::Stmt::Expr(statement)
                    if matches!(statement.expr, core::Expr::Try { .. })
                        && !(is_tail && returns_tail) =>
                {
                    let core::Expr::Try { value, span } = &statement.expr else {
                        unreachable!();
                    };
                    self.emit_try_statement(out, value, *span, indent, bindings)?;
                }
                core::Stmt::Expr(statement) if is_tail && returns_tail => {
                    self.emit_returning_expr(
                        out,
                        &statement.expr,
                        indent,
                        &bindings,
                        &binding_types,
                    )?;
                }
                core::Stmt::Expr(statement)
                    if matches!(statement.expr, core::Expr::Unit { .. }) => {}
                core::Stmt::Expr(statement) => {
                    out.push_str(indent);
                    out.push_str(&self.emit_expr(&statement.expr, &bindings)?);
                    out.push_str(";\n");
                }
                unsupported => {
                    if std::env::var_os("LUME_JAVA_DEBUG_STUBS").is_some() {
                        eprintln!(
                            "readable java cannot emit statement in '{}': {:?}",
                            self.function.name, unsupported
                        );
                    }
                    return None;
                }
            }
        }
        Some(())
    }

    #[allow(clippy::too_many_arguments)]
    fn emit_if_destructuring_binding(
        &self,
        out: &mut String,
        binding: &core::BindingStmt,
        indent: &str,
        bindings: &mut HashMap<String, String>,
        binding_types: &mut HashMap<String, ir::Type>,
        used_locals: &mut HashSet<ir::LocalId>,
    ) -> Option<()> {
        let [value @ core::Expr::If { .. }] = binding.values.as_slice() else {
            return None;
        };
        let value_ty = self
            .expr_type_with_binding_types(value, bindings, binding_types)
            .or_else(|| {
                (binding.destructure == Some(ast::DestructureKind::Tuple))
                    .then(|| {
                        binding
                            .bindings
                            .iter()
                            .map(|declared| {
                                self.source_binding_local(&declared.name, used_locals)
                                    .map(|local| local.ty.clone())
                            })
                            .collect::<Option<Vec<_>>>()
                            .map(ir::Type::Tuple)
                    })
                    .flatten()
            })?;
        let temp = format!("__destructure{}", binding.span.start);
        out.push_str(indent);
        out.push_str(&self.names.value_type(&value_ty));
        out.push(' ');
        out.push_str(&temp);
        out.push_str(" = ");
        out.push_str(&java_default_value(&value_ty));
        out.push_str(";\n");
        self.emit_assigning_expr(
            out,
            &temp,
            &value_ty,
            value,
            indent,
            bindings,
            binding_types,
            used_locals,
        )?;
        self.emit_flat_destructure(
            out,
            binding.destructure?,
            &binding.bindings,
            &temp,
            &value_ty,
            indent,
            bindings,
            binding_types,
            used_locals,
        )
    }

    fn emit_call_with_hoisted_try_args(
        &self,
        out: &mut String,
        expr: &core::Expr,
        indent: &str,
        bindings: &HashMap<String, String>,
    ) -> Option<Option<String>> {
        let core::Expr::Call {
            callee,
            args,
            style: _,
            span,
        } = expr
        else {
            return Some(None);
        };
        if !args
            .iter()
            .any(|arg| matches!(arg.value, core::Expr::Try { .. }))
        {
            return Some(None);
        }

        let mut rewritten = args.clone();
        let mut rewritten_bindings = bindings.clone();
        for arg in &mut rewritten {
            let core::Expr::Try {
                value,
                span: try_span,
            } = &arg.value
            else {
                continue;
            };
            let wrapped_ty = self.expr_type(value, &rewritten_bindings)?;
            let success_ty = lifted_success_type(&wrapped_ty)?;
            let java_temp = format!("__tryValue{}", try_span.start);
            let source_temp = format!("__try_arg_{}", try_span.start);
            out.push_str(indent);
            out.push_str(&self.names.value_type(&success_ty));
            out.push(' ');
            out.push_str(&java_temp);
            out.push_str(" = ");
            out.push_str(&java_default_value(&success_ty));
            out.push_str(";\n");
            self.emit_try_assignment(
                out,
                &java_temp,
                &success_ty,
                value,
                *try_span,
                indent,
                &rewritten_bindings,
            )?;
            rewritten_bindings.insert(source_temp.clone(), java_temp);
            arg.value = core::Expr::Identifier {
                name: source_temp,
                span: *try_span,
            };
        }

        Some(Some(self.emit_call(
            callee,
            &rewritten,
            *span,
            &rewritten_bindings,
        )?))
    }

    #[allow(clippy::too_many_arguments)]
    fn emit_short_circuit_try_binding(
        &self,
        out: &mut String,
        value: &core::Expr,
        java_name: &str,
        target_ty: &ir::Type,
        indent: &str,
        bindings: &HashMap<String, String>,
    ) -> Option<bool> {
        let core::Expr::Binary {
            left, op, right, ..
        } = value
        else {
            return Some(false);
        };
        let core::Expr::Try {
            value: wrapped,
            span,
        } = right.as_ref()
        else {
            return Some(false);
        };
        let initial = match op {
            ast::BinaryOp::And => "false",
            ast::BinaryOp::Or => "true",
            _ => return Some(false),
        };
        let condition = self.emit_expr(left, bindings)?;
        out.push_str(indent);
        out.push_str(&self.names.value_type(target_ty));
        out.push(' ');
        out.push_str(java_name);
        out.push_str(" = ");
        out.push_str(initial);
        out.push_str(";\n");
        out.push_str(indent);
        out.push_str("if (");
        if matches!(op, ast::BinaryOp::Or) {
            out.push_str("!(");
            out.push_str(&condition);
            out.push(')');
        } else {
            out.push_str(&condition);
        }
        out.push_str(") {\n");
        self.emit_try_assignment(
            out,
            java_name,
            target_ty,
            wrapped,
            *span,
            &format!("{indent}    "),
            bindings,
        )?;
        out.push_str(indent);
        out.push_str("}\n");
        Some(true)
    }

    #[allow(clippy::too_many_arguments)]
    fn emit_if_value_binding(
        &self,
        out: &mut String,
        binding: &ast::Binding,
        value: &core::Expr,
        indent: &str,
        bindings: &mut HashMap<String, String>,
        binding_types: &mut HashMap<String, ir::Type>,
        used_locals: &mut HashSet<ir::LocalId>,
    ) -> Option<()> {
        if binding.name == "_" {
            return None;
        }
        let local = self.source_binding_local(&binding.name, used_locals)?;
        used_locals.insert(local.id);
        let java_name = java_local_name(local);

        if matches!(value, core::Expr::If { .. })
            && let Some(inline_value) =
                self.emit_inline_value_expr(value, &local.ty, bindings, binding_types)
        {
            out.push_str(indent);
            out.push_str(&self.names.value_type(&local.ty));
            out.push(' ');
            out.push_str(&java_name);
            out.push_str(" = ");
            out.push_str(&inline_value);
            out.push_str(";\n");
            bindings.insert(binding.name.clone(), java_name);
            binding_types.insert(binding.name.clone(), local.ty.clone());
            return Some(());
        }

        out.push_str(indent);
        out.push_str(&self.names.value_type(&local.ty));
        out.push(' ');
        out.push_str(&java_name);
        out.push_str(" = ");
        out.push_str(&java_default_value(&local.ty));
        out.push_str(";\n");
        self.emit_assigning_expr(
            out,
            &java_name,
            &local.ty,
            value,
            indent,
            bindings,
            binding_types,
            used_locals,
        )?;
        bindings.insert(binding.name.clone(), java_name);
        binding_types.insert(binding.name.clone(), local.ty.clone());
        Some(())
    }

    fn emit_inline_value_expr(
        &self,
        expr: &core::Expr,
        target_ty: &ir::Type,
        bindings: &HashMap<String, String>,
        binding_types: &HashMap<String, ir::Type>,
    ) -> Option<String> {
        let core::Expr::If {
            condition_clauses,
            then_block,
            else_branch,
            ..
        } = expr
        else {
            return self.emit_expr_against(expr, bindings, target_ty);
        };
        let [core::Stmt::Expr(then_expr)] = then_block.statements.as_slice() else {
            return None;
        };
        let (condition, then_bindings, then_types) =
            self.emit_condition_clauses(None, condition_clauses, bindings, binding_types)?;
        let then_value =
            self.emit_inline_value_expr(&then_expr.expr, target_ty, &then_bindings, &then_types)?;
        let else_value = match else_branch.as_ref() {
            core::ElseExprBranch::If(next) => {
                self.emit_inline_value_expr(next, target_ty, bindings, binding_types)?
            }
            core::ElseExprBranch::Block(block) => {
                let [core::Stmt::Expr(else_expr)] = block.statements.as_slice() else {
                    return None;
                };
                self.emit_inline_value_expr(&else_expr.expr, target_ty, bindings, binding_types)?
            }
        };
        Some(format!("({condition}) ? {then_value} : {else_value}"))
    }

    fn emit_assigning_expr(
        &self,
        out: &mut String,
        target: &str,
        target_ty: &ir::Type,
        expr: &core::Expr,
        indent: &str,
        bindings: &HashMap<String, String>,
        binding_types: &HashMap<String, ir::Type>,
        used_locals: &mut HashSet<ir::LocalId>,
    ) -> Option<()> {
        match expr {
            core::Expr::If {
                condition_clauses,
                then_block,
                else_branch,
                ..
            } => {
                let (condition, then_bindings, then_types) =
                    self.emit_condition_clauses(None, condition_clauses, bindings, binding_types)?;
                out.push_str(indent);
                out.push_str("if (");
                out.push_str(&condition);
                out.push_str(") {\n");
                self.emit_assigning_block(
                    out,
                    target,
                    target_ty,
                    then_block,
                    &format!("{indent}    "),
                    &then_bindings,
                    &then_types,
                    used_locals,
                )?;
                out.push_str(indent);
                out.push_str("} else {\n");
                match else_branch.as_ref() {
                    core::ElseExprBranch::If(next) => self.emit_assigning_expr(
                        out,
                        target,
                        target_ty,
                        next,
                        &format!("{indent}    "),
                        bindings,
                        binding_types,
                        used_locals,
                    )?,
                    core::ElseExprBranch::Block(block) => self.emit_assigning_block(
                        out,
                        target,
                        target_ty,
                        block,
                        &format!("{indent}    "),
                        bindings,
                        binding_types,
                        used_locals,
                    )?,
                }
                out.push_str(indent);
                out.push_str("}\n");
                Some(())
            }
            core::Expr::Try { value, span } => {
                self.emit_try_assignment(out, target, target_ty, value, *span, indent, bindings)
            }
            _ => {
                out.push_str(indent);
                out.push_str(target);
                out.push_str(" = ");
                out.push_str(&self.emit_expr_against(expr, bindings, target_ty)?);
                out.push_str(";\n");
                Some(())
            }
        }
    }

    fn emit_assigning_block(
        &self,
        out: &mut String,
        target: &str,
        target_ty: &ir::Type,
        block: &core::Block,
        indent: &str,
        bindings: &HashMap<String, String>,
        binding_types: &HashMap<String, ir::Type>,
        used_locals: &mut HashSet<ir::LocalId>,
    ) -> Option<()> {
        let (tail, prefix) = block.statements.split_last()?;
        let mut branch_bindings = bindings.clone();
        let mut branch_types = binding_types.clone();
        if !prefix.is_empty() {
            self.emit_statement_block(
                out,
                &core::Block {
                    statements: prefix.to_vec(),
                    span: block.span,
                },
                indent,
                &mut branch_bindings,
                &mut branch_types,
                used_locals,
                false,
                0,
            )?;
        }
        let core::Stmt::Expr(statement) = tail else {
            return None;
        };
        self.emit_assigning_expr(
            out,
            target,
            target_ty,
            &statement.expr,
            indent,
            &branch_bindings,
            &branch_types,
            used_locals,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn emit_try_assignment(
        &self,
        out: &mut String,
        target: &str,
        target_ty: &ir::Type,
        value: &core::Expr,
        span: crate::source::Span,
        indent: &str,
        bindings: &HashMap<String, String>,
    ) -> Option<()> {
        let wrapped_ty = self.expr_type(value, bindings)?;
        let (case_type, accessor) = match &wrapped_ty {
            ir::Type::Named { name, .. } if name == "Option" => {
                ("lume.core.Option.Some<?>".to_string(), "value")
            }
            ir::Type::Named { name, .. } if name == "Result" => {
                ("lume.core.Result.Ok<?, ?>".to_string(), "value")
            }
            ir::Type::Named { name, .. } if name == "Either" => {
                ("lume.core.Either.Right<?, ?>".to_string(), "value")
            }
            _ => return None,
        };
        let wrapped = format!("__try{}", span.start);
        let success = format!("__success{}", span.start);
        out.push_str(indent);
        out.push_str("var ");
        out.push_str(&wrapped);
        out.push_str(" = ");
        out.push_str(&self.emit_expr(value, bindings)?);
        out.push_str(";\n");
        out.push_str(indent);
        out.push_str("if (!(");
        out.push_str(&wrapped);
        out.push_str(" instanceof ");
        out.push_str(&case_type);
        out.push(' ');
        out.push_str(&success);
        out.push_str(")) {\n");
        out.push_str(indent);
        out.push_str("    return (");
        out.push_str(&self.names.value_type(&self.function.return_ty));
        out.push_str(") (Object) ");
        out.push_str(&wrapped);
        out.push_str(";\n");
        out.push_str(indent);
        out.push_str("}\n");
        out.push_str(indent);
        out.push_str(target);
        out.push_str(" = (");
        out.push_str(&self.names.value_type(target_ty));
        out.push_str(") ");
        out.push_str(&success);
        out.push('.');
        out.push_str(accessor);
        out.push_str("();\n");
        Some(())
    }

    fn emit_try_statement(
        &self,
        out: &mut String,
        value: &core::Expr,
        span: crate::source::Span,
        indent: &str,
        bindings: &HashMap<String, String>,
    ) -> Option<()> {
        let wrapped_ty = self.expr_type(value, bindings)?;
        let success_case = match &wrapped_ty {
            ir::Type::Named { name, .. } if name == "Option" => "lume.core.Option.Some<?>",
            ir::Type::Named { name, .. } if name == "Result" => "lume.core.Result.Ok<?, ?>",
            ir::Type::Named { name, .. } if name == "Either" => "lume.core.Either.Right<?, ?>",
            _ => return None,
        };
        let wrapped = format!("__try{}", span.start);
        out.push_str(indent);
        out.push_str("var ");
        out.push_str(&wrapped);
        out.push_str(" = ");
        out.push_str(&self.emit_expr(value, bindings)?);
        out.push_str(";\n");
        out.push_str(indent);
        out.push_str("if (!(");
        out.push_str(&wrapped);
        out.push_str(" instanceof ");
        out.push_str(success_case);
        out.push_str(")) {\n");
        out.push_str(indent);
        out.push_str("    return (");
        out.push_str(&self.names.value_type(&self.function.return_ty));
        out.push_str(") (Object) ");
        out.push_str(&wrapped);
        out.push_str(";\n");
        out.push_str(indent);
        out.push_str("}\n");
        Some(())
    }

    #[allow(clippy::too_many_arguments)]
    fn emit_try_binding(
        &self,
        out: &mut String,
        binding: &ast::Binding,
        value: &core::Expr,
        span: crate::source::Span,
        indent: &str,
        bindings: &mut HashMap<String, String>,
        binding_types: &mut HashMap<String, ir::Type>,
        used_locals: &mut HashSet<ir::LocalId>,
    ) -> Option<()> {
        if binding.name == "_" {
            return None;
        }
        let wrapped_ty = self.expr_type(value, bindings)?;
        let success_ty = lifted_success_type(&wrapped_ty)?;
        let (case_type, accessor) = match &wrapped_ty {
            ir::Type::Named { name, .. } if name == "Option" => {
                ("lume.core.Option.Some<?>".to_string(), "value")
            }
            ir::Type::Named { name, .. } if name == "Result" => {
                ("lume.core.Result.Ok<?, ?>".to_string(), "value")
            }
            ir::Type::Named { name, .. } if name == "Either" => {
                ("lume.core.Either.Right<?, ?>".to_string(), "value")
            }
            _ => return None,
        };
        let local = self.source_binding_local(&binding.name, used_locals)?;
        used_locals.insert(local.id);
        let local_ty = if matches!(local.ty, ir::Type::Unknown) {
            success_ty
        } else {
            local.ty.clone()
        };
        let java_name = java_local_name(local);
        let wrapped = format!("__try{}", span.start);
        let success = format!("__success{}", span.start);

        out.push_str(indent);
        out.push_str("var ");
        out.push_str(&wrapped);
        out.push_str(" = ");
        out.push_str(&self.emit_expr(value, bindings)?);
        out.push_str(";\n");
        out.push_str(indent);
        out.push_str("if (!(");
        out.push_str(&wrapped);
        out.push_str(" instanceof ");
        out.push_str(&case_type);
        out.push(' ');
        out.push_str(&success);
        out.push_str(")) {\n");
        out.push_str(indent);
        out.push_str("    return (");
        out.push_str(&self.names.value_type(&self.function.return_ty));
        out.push_str(") (Object) ");
        out.push_str(&wrapped);
        out.push_str(";\n");
        out.push_str(indent);
        out.push_str("}\n");
        out.push_str(indent);
        out.push_str(&self.names.value_type(&local_ty));
        out.push(' ');
        out.push_str(&java_name);
        out.push_str(" = (");
        out.push_str(&self.names.value_type(&local_ty));
        out.push_str(") ");
        out.push_str(&success);
        out.push('.');
        out.push_str(accessor);
        out.push_str("();\n");

        bindings.insert(binding.name.clone(), java_name);
        binding_types.insert(binding.name.clone(), local_ty);
        Some(())
    }

    #[allow(clippy::too_many_arguments)]
    fn emit_let_else_statement(
        &self,
        out: &mut String,
        statement: &core::LetElseStmt,
        indent: &str,
        bindings: &mut HashMap<String, String>,
        binding_types: &mut HashMap<String, ir::Type>,
        used_locals: &mut HashSet<ir::LocalId>,
        loop_depth: usize,
    ) -> Option<()> {
        if statement.clauses.is_empty() {
            return self.emit_let_else_clause(
                out,
                &statement.pattern,
                &statement.value,
                &statement.else_block,
                statement.span.start,
                indent,
                bindings,
                binding_types,
                used_locals,
                loop_depth,
            );
        }

        for (index, clause) in statement.clauses.iter().enumerate() {
            self.emit_let_else_clause(
                out,
                &clause.pattern,
                &clause.value,
                &statement.else_block,
                statement.span.start + index,
                indent,
                bindings,
                binding_types,
                used_locals,
                loop_depth,
            )?;
        }
        Some(())
    }

    #[allow(clippy::too_many_arguments)]
    fn emit_let_else_clause(
        &self,
        out: &mut String,
        pattern: &ast::Pattern,
        value: &core::Expr,
        else_block: &core::Block,
        unique: usize,
        indent: &str,
        bindings: &mut HashMap<String, String>,
        binding_types: &mut HashMap<String, ir::Type>,
        used_locals: &mut HashSet<ir::LocalId>,
        loop_depth: usize,
    ) -> Option<()> {
        let value_ty = self.expr_type_with_binding_types(value, bindings, binding_types)?;
        let value_expr = self.emit_expr(value, bindings)?;
        let value_local = format!("__let{unique}");
        out.push_str(indent);
        out.push_str("var ");
        out.push_str(&value_local);
        out.push_str(" = ");
        out.push_str(&value_expr);
        out.push_str(";\n");

        let matched = self.match_case_pattern(
            pattern,
            &value_local,
            &value_ty,
            unique,
            bindings,
            binding_types,
        )?;
        let mut new_bindings = matched
            .bindings
            .iter()
            .filter(|(name, value)| bindings.get(*name) != Some(*value))
            .map(|(name, value)| (name.clone(), value.clone()))
            .collect::<Vec<_>>();
        new_bindings.sort_by(|left, right| left.0.cmp(&right.0));

        let mut materialized = Vec::new();
        for (name, value) in new_bindings {
            let local = self.source_binding_local(&name, used_locals)?;
            used_locals.insert(local.id);
            let java_name = java_local_name(local);
            let ty = matched.binding_types.get(&name)?.clone();
            materialized.push((name, java_name, ty, value));
        }

        out.push_str(indent);
        out.push_str("if (!(");
        out.push_str(&matched.condition);
        out.push_str(")) {\n");
        let mut fallback_bindings = bindings.clone();
        let mut fallback_types = binding_types.clone();
        self.emit_statement_block(
            out,
            else_block,
            &format!("{indent}    "),
            &mut fallback_bindings,
            &mut fallback_types,
            used_locals,
            false,
            loop_depth,
        )?;
        out.push_str(indent);
        out.push_str("}\n");

        for (_, java_name, ty, value) in &materialized {
            out.push_str(indent);
            out.push_str(&self.names.value_type(ty));
            out.push(' ');
            out.push_str(java_name);
            out.push_str(" = ");
            out.push_str(value);
            out.push_str(";\n");
        }

        for (name, java_name, ty, _) in materialized {
            bindings.insert(name.clone(), java_name);
            binding_types.insert(name, ty);
        }
        Some(())
    }

    #[allow(clippy::too_many_arguments)]
    fn emit_match_statement(
        &self,
        out: &mut String,
        statement: &core::MatchStmt,
        indent: &str,
        bindings: &HashMap<String, String>,
        binding_types: &HashMap<String, ir::Type>,
        used_locals: &mut HashSet<ir::LocalId>,
        loop_depth: usize,
        branches_return: bool,
    ) -> Option<()> {
        let value_ty = self.expr_type(&statement.value, bindings)?;
        let value = self.emit_expr(&statement.value, bindings)?;
        let match_local = format!("__match{}", statement.span.start);
        out.push_str(indent);
        out.push_str("var ");
        out.push_str(&match_local);
        out.push_str(" = ");
        out.push_str(&value);
        out.push_str(";\n");

        for (index, case) in statement.cases.iter().enumerate() {
            let matched = self.match_case_pattern(
                &case.pattern,
                &match_local,
                &value_ty,
                statement.span.start + index,
                bindings,
                binding_types,
            )
            .or_else(|| {
                if std::env::var_os("LUME_JAVA_DEBUG_STUBS").is_some() {
                    eprintln!(
                        "readable java cannot emit match pattern in '{}': pattern={:?}, value_type={value_ty:?}",
                        self.function.name, case.pattern
                    );
                }
                None
            })?;
            let condition = match &case.guard {
                Some(guard) => format!(
                    "({}) && ({})",
                    matched.condition,
                    self.emit_expr(guard, &matched.bindings)?
                ),
                None => matched.condition,
            };
            out.push_str(indent);
            if index > 0 {
                out.push_str("else ");
            }
            out.push_str("if (");
            out.push_str(&condition);
            out.push_str(") {\n");
            let branch_indent = format!("{indent}    ");
            let mut branch_bindings = matched.bindings;
            let mut branch_types = matched.binding_types;
            match &case.body {
                core::MatchCaseBody::Expr(expr) if branches_return => self.emit_returning_expr(
                    out,
                    expr,
                    &branch_indent,
                    &branch_bindings,
                    &branch_types,
                )?,
                core::MatchCaseBody::Expr(core::Expr::Unit { .. }) => {}
                core::MatchCaseBody::Expr(expr) => {
                    out.push_str(&branch_indent);
                    out.push_str(&self.emit_expr(expr, &branch_bindings)?);
                    out.push_str(";\n");
                }
                core::MatchCaseBody::Block(block) => self.emit_statement_block(
                    out,
                    block,
                    &branch_indent,
                    &mut branch_bindings,
                    &mut branch_types,
                    used_locals,
                    branches_return,
                    loop_depth,
                )?,
            }
            out.push_str(indent);
            out.push_str("} ");
        }
        if statement.cases.is_empty() {
            return None;
        }
        if statement.partial {
            out.push_str("\n");
        } else {
            out.push_str("else {\n");
            out.push_str(&format!(
                "{indent}    throw new IllegalStateException(\"non-exhaustive Lume match\");\n"
            ));
            out.push_str(indent);
            out.push_str("}\n");
        }
        Some(())
    }

    #[allow(clippy::too_many_arguments)]
    fn emit_for_statement(
        &self,
        out: &mut String,
        statement: &core::ForStmt,
        indent: &str,
        bindings: &HashMap<String, String>,
        binding_types: &HashMap<String, ir::Type>,
        used_locals: &mut HashSet<ir::LocalId>,
        loop_depth: usize,
    ) -> Option<()> {
        let [generator] = statement.bindings.as_slice() else {
            return None;
        };
        if generator.pattern.is_some() || !generator.values.is_empty() {
            return None;
        }
        if generator.destructure.is_none() && generator.bindings.len() != 1 {
            return None;
        }
        let iterable_source = generator.iterable.as_ref()?;
        let item_ty = self
            .iterable_item_type(iterable_source, bindings, binding_types)
            .or_else(|| {
                generator
                    .bindings
                    .iter()
                    .find(|binding| binding.name != "_")
                    .and_then(|binding| self.source_binding_local(&binding.name, used_locals))
                    .map(|local| local.ty.clone())
            })?;
        let iterable = self.emit_expr(iterable_source, bindings)?;
        let iterator = format!("__iterator{}", statement.span.start);

        out.push_str(indent);
        out.push_str("lume.core.LumeIterator<?> ");
        out.push_str(&iterator);
        out.push_str(" = lume.core.LumeIterator.from(");
        out.push_str(&iterable);
        out.push_str(");\n");
        out.push_str(indent);
        out.push_str("while (");
        out.push_str(&iterator);
        out.push_str(".hasNext()) {\n");

        let mut body_bindings = bindings.clone();
        let mut body_types = binding_types.clone();
        if let Some(destructure) = generator.destructure {
            let item = format!("__item{}", generator.span.start);
            let java_type = self.names.value_type(&item_ty);
            out.push_str(&format!(
                "{indent}    {java_type} {item} = ({java_type}) {iterator}.next();\n"
            ));
            self.emit_flat_destructure(
                out,
                destructure,
                &generator.bindings,
                &item,
                &item_ty,
                &format!("{indent}    "),
                &mut body_bindings,
                &mut body_types,
                used_locals,
            )?;
        } else {
            let binding = &generator.bindings[0];
            if binding.name == "_" {
                out.push_str(&format!("{indent}    {iterator}.next();\n"));
            } else {
                let local = self.source_binding_local(&binding.name, used_locals)?;
                used_locals.insert(local.id);
                let java_name = java_local_name(local);
                let binding_ty = if matches!(local.ty, ir::Type::Unknown) {
                    item_ty
                } else {
                    local.ty.clone()
                };
                let java_type = self.names.value_type(&binding_ty);
                out.push_str(&format!(
                    "{indent}    {java_type} {java_name} = ({java_type}) {iterator}.next();\n"
                ));
                body_bindings.insert(binding.name.clone(), java_name);
                body_types.insert(binding.name.clone(), binding_ty);
            }
        }

        self.emit_statement_block(
            out,
            &statement.body,
            &format!("{indent}    "),
            &mut body_bindings,
            &mut body_types,
            used_locals,
            false,
            loop_depth + 1,
        )?;
        out.push_str(indent);
        out.push_str("}\n");
        Some(())
    }

    fn iterable_item_type(
        &self,
        iterable: &core::Expr,
        bindings: &HashMap<String, String>,
        binding_types: &HashMap<String, ir::Type>,
    ) -> Option<ir::Type> {
        if matches!(
            iterable,
            core::Expr::Call { callee, .. }
                if matches!(callee.as_ref(), core::Expr::Identifier { name, .. } if name == "Range")
        ) {
            return Some(ir::Type::Int);
        }
        let iterable_ty = self
            .expr_type(iterable, bindings)
            .or_else(|| match iterable {
                core::Expr::Identifier { name, .. } => {
                    binding_types.get(name).cloned().or_else(|| {
                        self.function
                            .params
                            .iter()
                            .filter_map(|param| self.function.locals.get(param.0))
                            .find(|local| local.name == *name)
                            .map(|local| local.ty.clone())
                    })
                }
                core::Expr::ListLiteral { items, .. } => {
                    let element = items
                        .iter()
                        .filter_map(source_literal_type)
                        .find(|ty| !matches!(ty, ir::Type::Unknown))
                        .unwrap_or(ir::Type::Unknown);
                    Some(ir::Type::list(element))
                }
                core::Expr::Call {
                    callee,
                    args,
                    style: core::CallStyle::Paren,
                    ..
                } if args.is_empty() => {
                    let core::Expr::Member { receiver, name, .. } = callee.as_ref() else {
                        return None;
                    };
                    if name != "zipWithIndex" {
                        return None;
                    }
                    let receiver_ty =
                        self.expr_type(receiver, bindings)
                            .or_else(|| match receiver.as_ref() {
                                core::Expr::Identifier { name, .. } => {
                                    binding_types.get(name).cloned()
                                }
                                _ => None,
                            })?;
                    let item_ty = iterable_item_type(&receiver_ty)?;
                    Some(ir::Type::list(ir::Type::Tuple(vec![
                        item_ty,
                        ir::Type::Int,
                    ])))
                }
                _ => None,
            })?;
        match iterable_ty {
            ir::Type::Named { name, args }
                if matches!(
                    name.as_str(),
                    "Vector" | "Iterable" | "Iterator" | "Array" | "LinkedList" | "Set"
                ) && args.len() == 1 =>
            {
                args.into_iter().next()
            }
            ir::Type::Named { name, args } if name == "Map" && args.len() == 2 => {
                Some(ir::Type::Tuple(args))
            }
            ir::Type::Named { name, args }
                if matches!(name.as_str(), "IntRange" | "Range") && args.is_empty() =>
            {
                Some(ir::Type::Int)
            }
            _ => None,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn emit_flat_destructuring_binding(
        &self,
        out: &mut String,
        destructure: ast::DestructureKind,
        declared_bindings: &[ast::Binding],
        values: &[core::Expr],
        span: crate::source::Span,
        indent: &str,
        bindings: &mut HashMap<String, String>,
        binding_types: &mut HashMap<String, ir::Type>,
        used_locals: &mut HashSet<ir::LocalId>,
    ) -> Option<()> {
        let [value] = values else {
            return None;
        };
        let value_ty = self.expr_type_with_binding_types(value, bindings, binding_types)?;
        let temp = format!("__destructure{}", span.start);
        out.push_str(indent);
        out.push_str(&self.names.value_type(&value_ty));
        out.push(' ');
        out.push_str(&temp);
        out.push_str(" = ");
        out.push_str(&self.emit_expr_against(value, bindings, &value_ty)?);
        out.push_str(";\n");
        self.emit_flat_destructure(
            out,
            destructure,
            declared_bindings,
            &temp,
            &value_ty,
            indent,
            bindings,
            binding_types,
            used_locals,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn emit_flat_destructure(
        &self,
        out: &mut String,
        destructure: ast::DestructureKind,
        declared_bindings: &[ast::Binding],
        value: &str,
        value_ty: &ir::Type,
        indent: &str,
        bindings: &mut HashMap<String, String>,
        binding_types: &mut HashMap<String, ir::Type>,
        used_locals: &mut HashSet<ir::LocalId>,
    ) -> Option<()> {
        let fields = match destructure {
            ast::DestructureKind::Tuple => {
                let ir::Type::Tuple(items) = value_ty else {
                    return None;
                };
                if items.len() != declared_bindings.len() {
                    return None;
                }
                declared_bindings
                    .iter()
                    .zip(items)
                    .enumerate()
                    .map(|(index, (binding, ty))| {
                        let accessor = tuple_accessor_name(&format!("_{}", index + 1))?;
                        Some((binding, ty.clone(), format!("{value}.{accessor}()")))
                    })
                    .collect::<Option<Vec<_>>>()?
            }
            ast::DestructureKind::Record => {
                let ir::Type::Named { name, .. } = value_ty else {
                    return None;
                };
                let ty = self.bundle.ir.types.iter().find(|ty| ty.name == *name)?;
                declared_bindings
                    .iter()
                    .map(|binding| {
                        let field_name = binding.field_name.as_deref().unwrap_or(&binding.name);
                        let field = ty.fields.iter().find(|field| field.name == field_name)?;
                        let member = java_member_name(field_name);
                        let access = if ty.kind == TypeKind::Record
                            || ty.kind == TypeKind::Interface
                            || is_anonymous_object_type(ty)
                        {
                            format!("{value}.{member}()")
                        } else {
                            format!("{value}.{member}")
                        };
                        Some((binding, field.ty.clone(), access))
                    })
                    .collect::<Option<Vec<_>>>()?
            }
        };

        for (binding, field_ty, access) in fields {
            if binding.name == "_" {
                continue;
            }
            let local = self.source_binding_local(&binding.name, used_locals)?;
            used_locals.insert(local.id);
            let local_ty = if matches!(local.ty, ir::Type::Unknown) {
                field_ty
            } else {
                local.ty.clone()
            };
            let java_name = java_local_name(local);
            let java_type = self.names.value_type(&local_ty);
            out.push_str(indent);
            out.push_str(&java_type);
            out.push(' ');
            out.push_str(&java_name);
            out.push_str(" = (");
            out.push_str(&java_type);
            out.push_str(") ");
            out.push_str(&access);
            out.push_str(";\n");
            bindings.insert(binding.name.clone(), java_name);
            binding_types.insert(binding.name.clone(), local_ty);
        }
        Some(())
    }

    #[allow(clippy::too_many_arguments)]
    fn emit_if_statement(
        &self,
        out: &mut String,
        statement: &core::IfStmt,
        indent: &str,
        bindings: &HashMap<String, String>,
        binding_types: &HashMap<String, ir::Type>,
        used_locals: &mut HashSet<ir::LocalId>,
        loop_depth: usize,
        branches_return: bool,
    ) -> Option<()> {
        if !statement.bindings.is_empty() || statement.binding_value.is_some() {
            if std::env::var_os("LUME_JAVA_DEBUG_STUBS").is_some() {
                eprintln!(
                    "readable java cannot emit legacy if bindings in '{}': {:?}",
                    self.function.name, statement
                );
            }
            return None;
        }
        let mut conditions = Vec::new();
        let mut then_bindings = bindings.clone();
        let mut then_types = binding_types.clone();
        if statement.condition.is_some() || !statement.condition_clauses.is_empty() {
            let (condition, next_bindings, next_types) = self.emit_condition_clauses(
                statement.condition.as_ref(),
                &statement.condition_clauses,
                &then_bindings,
                &then_types,
            )?;
            conditions.push(condition);
            then_bindings = next_bindings;
            then_types = next_types;
        }
        if let (Some(pattern), Some(value)) = (&statement.pattern, &statement.pattern_value) {
            let (condition, next_bindings, next_types) = self.emit_refutable_condition(
                pattern,
                value,
                statement.span.start,
                &then_bindings,
                &then_types,
            )?;
            conditions.push(condition);
            then_bindings = next_bindings;
            then_types = next_types;
        } else if statement.pattern.is_some() || statement.pattern_value.is_some() {
            return None;
        }
        for (index, clause) in statement.pattern_clauses.iter().enumerate() {
            let (condition, next_bindings, next_types) = self.emit_refutable_condition(
                &clause.pattern,
                &clause.value,
                clause.span.start + index,
                &then_bindings,
                &then_types,
            )?;
            conditions.push(condition);
            then_bindings = next_bindings;
            then_types = next_types;
        }
        let condition = (!conditions.is_empty()).then(|| conditions.join(" && "))?;
        if branches_return && statement.else_branch.is_none() {
            return None;
        }
        out.push_str(indent);
        out.push_str("if (");
        out.push_str(&condition);
        out.push_str(") {\n");
        self.emit_statement_block(
            out,
            &statement.then_block,
            &format!("{indent}    "),
            &mut then_bindings,
            &mut then_types,
            used_locals,
            branches_return,
            loop_depth,
        )?;
        out.push_str(indent);
        match &statement.else_branch {
            Some(core::ElseBranch::If(next)) => {
                out.push_str("} else ");
                self.emit_if_statement_after_else(
                    out,
                    next,
                    indent,
                    bindings,
                    binding_types,
                    used_locals,
                    loop_depth,
                    branches_return,
                )?;
            }
            Some(core::ElseBranch::Block(block)) => {
                out.push_str("} else {\n");
                let mut else_bindings = bindings.clone();
                let mut else_types = binding_types.clone();
                self.emit_statement_block(
                    out,
                    block,
                    &format!("{indent}    "),
                    &mut else_bindings,
                    &mut else_types,
                    used_locals,
                    branches_return,
                    loop_depth,
                )?;
                out.push_str(indent);
                out.push_str("}\n");
            }
            None => out.push_str("}\n"),
        }
        Some(())
    }

    fn emit_refutable_condition(
        &self,
        pattern: &ast::Pattern,
        value: &core::Expr,
        unique: usize,
        bindings: &HashMap<String, String>,
        binding_types: &HashMap<String, ir::Type>,
    ) -> Option<(String, HashMap<String, String>, HashMap<String, ir::Type>)> {
        let value_ty = self.expr_type(value, bindings)?;
        let value = self.emit_expr(value, bindings)?;
        let matched =
            self.match_case_pattern(pattern, &value, &value_ty, unique, bindings, binding_types)?;
        Some((matched.condition, matched.bindings, matched.binding_types))
    }

    #[allow(clippy::too_many_arguments)]
    fn emit_if_statement_after_else(
        &self,
        out: &mut String,
        statement: &core::IfStmt,
        indent: &str,
        bindings: &HashMap<String, String>,
        binding_types: &HashMap<String, ir::Type>,
        used_locals: &mut HashSet<ir::LocalId>,
        loop_depth: usize,
        branches_return: bool,
    ) -> Option<()> {
        let mut nested = String::new();
        self.emit_if_statement(
            &mut nested,
            statement,
            indent,
            bindings,
            binding_types,
            used_locals,
            loop_depth,
            branches_return,
        )?;
        out.push_str(nested.trim_start());
        Some(())
    }

    #[allow(clippy::too_many_arguments)]
    fn emit_while_statement(
        &self,
        out: &mut String,
        statement: &core::WhileStmt,
        indent: &str,
        bindings: &HashMap<String, String>,
        binding_types: &HashMap<String, ir::Type>,
        used_locals: &mut HashSet<ir::LocalId>,
        loop_depth: usize,
    ) -> Option<()> {
        let (condition, mut body_bindings, mut body_types) = self.emit_condition_clauses(
            None,
            &statement.condition_clauses,
            bindings,
            binding_types,
        )?;
        out.push_str(indent);
        out.push_str("while (");
        out.push_str(&condition);
        out.push_str(") {\n");
        self.emit_statement_block(
            out,
            &statement.body,
            &format!("{indent}    "),
            &mut body_bindings,
            &mut body_types,
            used_locals,
            false,
            loop_depth + 1,
        )?;
        out.push_str(indent);
        out.push_str("}\n");
        Some(())
    }

    fn emit_condition_clauses(
        &self,
        legacy_condition: Option<&core::Expr>,
        clauses: &[core::IfConditionClause],
        bindings: &HashMap<String, String>,
        binding_types: &HashMap<String, ir::Type>,
    ) -> Option<(String, HashMap<String, String>, HashMap<String, ir::Type>)> {
        if let Some(condition) = legacy_condition {
            if !clauses.is_empty() {
                return None;
            }
            return Some((
                self.emit_expr(condition, bindings)?,
                bindings.clone(),
                binding_types.clone(),
            ));
        }

        let mut conditions = Vec::new();
        let mut clause_bindings = bindings.clone();
        let mut clause_types = binding_types.clone();
        for (index, clause) in clauses.iter().enumerate() {
            match clause {
                core::IfConditionClause::Expr(condition) => {
                    conditions.push(self.emit_expr(condition, &clause_bindings)?);
                }
                core::IfConditionClause::Let(clause) => {
                    let value_ty = match self.expr_type_with_binding_types(
                        &clause.value,
                        &clause_bindings,
                        &clause_types,
                    ) {
                        Some(value_ty) => value_ty,
                        None => {
                            if std::env::var_os("LUME_JAVA_DEBUG_STUBS").is_some() {
                                eprintln!(
                                    "readable java cannot determine refutable condition value type in '{}': span={:?}",
                                    self.function.name,
                                    clause.value.span()
                                );
                            }
                            return None;
                        }
                    };
                    let value = match self.emit_expr(&clause.value, &clause_bindings) {
                        Some(value) => value,
                        None => {
                            if std::env::var_os("LUME_JAVA_DEBUG_STUBS").is_some() {
                                eprintln!(
                                    "readable java cannot emit refutable condition value in '{}'",
                                    self.function.name
                                );
                            }
                            return None;
                        }
                    };
                    let matched = match self.match_case_pattern(
                        &clause.pattern,
                        &value,
                        &value_ty,
                        clause.span.start + index,
                        &clause_bindings,
                        &clause_types,
                    ) {
                        Some(matched) => matched,
                        None => {
                            if std::env::var_os("LUME_JAVA_DEBUG_STUBS").is_some() {
                                eprintln!(
                                    "readable java cannot emit refutable condition pattern in '{}': pattern={:?}, value_type={value_ty:?}",
                                    self.function.name, clause.pattern
                                );
                            }
                            return None;
                        }
                    };
                    conditions.push(matched.condition);
                    clause_bindings = matched.bindings;
                    clause_types = matched.binding_types;
                }
            }
        }
        if conditions.is_empty() {
            return None;
        }
        Some((conditions.join(" && "), clause_bindings, clause_types))
    }

    fn emit_assignment_target(
        &self,
        target: &core::Expr,
        bindings: &HashMap<String, String>,
    ) -> Option<String> {
        match target {
            core::Expr::Identifier { name, .. } => bindings
                .get(name)
                .cloned()
                .or_else(|| self.param_reference(name))
                .or_else(|| self.implicit_field_reference(name)),
            core::Expr::Member { receiver, name, .. } if matches!(receiver.as_ref(), core::Expr::Identifier { name, .. } if name == "this") => {
                Some(format!("this.{}", java_member_name(name)))
            }
            _ => None,
        }
    }

    fn assignment_target_type(
        &self,
        target: &core::Expr,
        bindings: &HashMap<String, String>,
        binding_types: &HashMap<String, ir::Type>,
    ) -> Option<ir::Type> {
        match target {
            core::Expr::Identifier { name, .. } => binding_types
                .get(name)
                .cloned()
                .or_else(|| self.expr_type(target, bindings)),
            core::Expr::Member { receiver, name, .. }
                if matches!(
                    receiver.as_ref(),
                    core::Expr::Identifier { name, .. } if name == "this"
                ) =>
            {
                self.owner.and_then(|owner| {
                    owner
                        .fields
                        .iter()
                        .find(|field| field.name == *name)
                        .map(|field| field.ty.clone())
                })
            }
            _ => self.expr_type(target, bindings),
        }
    }

    fn source_binding_local(
        &self,
        name: &str,
        used_locals: &HashSet<ir::LocalId>,
    ) -> Option<&'a ir::Local> {
        self.function.locals.iter().find(|local| {
            local.name == name
                && local.kind == ir::LocalKind::Binding
                && !used_locals.contains(&local.id)
        })
    }

    fn emit_returning_expr(
        &self,
        out: &mut String,
        expr: &core::Expr,
        indent: &str,
        bindings: &HashMap<String, String>,
        binding_types: &HashMap<String, ir::Type>,
    ) -> Option<()> {
        match expr {
            core::Expr::If {
                condition_clauses,
                then_block,
                else_branch,
                ..
            } => self.emit_if_return(
                out,
                condition_clauses,
                then_block,
                else_branch,
                indent,
                bindings,
                binding_types,
            ),
            core::Expr::Match {
                partial: false,
                value,
                cases,
                span,
            } => self.emit_match_return(out, value, cases, *span, indent, bindings, binding_types),
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

    fn emit_if_return(
        &self,
        out: &mut String,
        condition_clauses: &[core::IfConditionClause],
        then_block: &core::Block,
        else_branch: &core::ElseExprBranch,
        indent: &str,
        bindings: &HashMap<String, String>,
        binding_types: &HashMap<String, ir::Type>,
    ) -> Option<()> {
        let (condition, then_bindings, then_types) =
            self.emit_condition_clauses(None, condition_clauses, bindings, binding_types)?;

        out.push_str(indent);
        out.push_str("if (");
        out.push_str(&condition);
        out.push_str(") {\n");
        self.emit_simple_returning_block(
            out,
            then_block,
            &format!("{indent}    "),
            &then_bindings,
            &then_types,
        )?;
        out.push_str(indent);
        out.push_str("} else {\n");
        match else_branch {
            core::ElseExprBranch::If(expr) => self.emit_returning_expr(
                out,
                expr,
                &format!("{indent}    "),
                bindings,
                binding_types,
            )?,
            core::ElseExprBranch::Block(block) => self.emit_simple_returning_block(
                out,
                block,
                &format!("{indent}    "),
                bindings,
                binding_types,
            )?,
        }
        out.push_str(indent);
        out.push_str("}\n");
        Some(())
    }

    fn emit_simple_returning_block(
        &self,
        out: &mut String,
        block: &core::Block,
        indent: &str,
        bindings: &HashMap<String, String>,
        binding_types: &HashMap<String, ir::Type>,
    ) -> Option<()> {
        let [statement] = block.statements.as_slice() else {
            return None;
        };
        match statement {
            core::Stmt::Expr(statement) => {
                self.emit_returning_expr(out, &statement.expr, indent, bindings, binding_types)
            }
            core::Stmt::Return(statement) => {
                out.push_str(indent);
                out.push_str("return");
                if let Some(value) = &statement.value {
                    out.push(' ');
                    out.push_str(&self.emit_expr_against(
                        value,
                        bindings,
                        &self.function.return_ty,
                    )?);
                }
                out.push_str(";\n");
                Some(())
            }
            _ => None,
        }
    }

    fn emit_match_return(
        &self,
        out: &mut String,
        value: &core::Expr,
        cases: &[core::MatchCase],
        span: crate::source::Span,
        indent: &str,
        bindings: &HashMap<String, String>,
        binding_types: &HashMap<String, ir::Type>,
    ) -> Option<()> {
        let value_ty = self.expr_type(value, bindings).or_else(|| {
            if std::env::var_os("LUME_JAVA_DEBUG_STUBS").is_some() {
                eprintln!(
                    "readable java cannot infer returning match value type in '{}': {value:?}",
                    self.function.name
                );
            }
            None
        })?;
        let value = self.emit_expr(value, bindings).or_else(|| {
            if std::env::var_os("LUME_JAVA_DEBUG_STUBS").is_some() {
                eprintln!(
                    "readable java cannot emit returning match value in '{}': {value:?}",
                    self.function.name
                );
            }
            None
        })?;
        let match_local = format!("__match{}", span.start);
        out.push_str(indent);
        out.push_str("var ");
        out.push_str(&match_local);
        out.push_str(" = ");
        out.push_str(&value);
        out.push_str(";\n");
        for (index, case) in cases.iter().enumerate() {
            let matched = self.match_case_pattern(
                &case.pattern,
                &match_local,
                &value_ty,
                span.start + index,
                bindings,
                binding_types,
            )
            .or_else(|| {
                if std::env::var_os("LUME_JAVA_DEBUG_STUBS").is_some() {
                    eprintln!(
                        "readable java cannot emit returning match pattern in '{}': pattern={:?}, value_type={value_ty:?}",
                        self.function.name, case.pattern
                    );
                }
                None
            })?;
            let condition = match &case.guard {
                Some(guard) => format!(
                    "({}) && ({})",
                    matched.condition,
                    self.emit_expr(guard, &matched.bindings)?
                ),
                None => matched.condition.clone(),
            };
            out.push_str(indent);
            out.push_str("if (");
            out.push_str(&condition);
            out.push_str(") {\n");
            self.emit_match_case_body(
                out,
                &case.body,
                &format!("{indent}    "),
                &matched.bindings,
                &matched.binding_types,
            )
            .or_else(|| {
                if std::env::var_os("LUME_JAVA_DEBUG_STUBS").is_some() {
                    eprintln!(
                        "readable java cannot emit returning match body in '{}': {:?}",
                        self.function.name, case.body
                    );
                }
                None
            })?;
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
        body: &core::MatchCaseBody,
        indent: &str,
        bindings: &HashMap<String, String>,
        binding_types: &HashMap<String, ir::Type>,
    ) -> Option<()> {
        match body {
            core::MatchCaseBody::Expr(expr) => {
                self.emit_returning_expr(out, expr, indent, bindings, binding_types)
            }
            core::MatchCaseBody::Block(_) => None,
        }
    }

    fn match_case_pattern(
        &self,
        pattern: &ast::Pattern,
        value: &str,
        value_ty: &ir::Type,
        index: usize,
        parent_bindings: &HashMap<String, String>,
        parent_binding_types: &HashMap<String, ir::Type>,
    ) -> Option<MatchedCase> {
        match pattern {
            ast::Pattern::Wildcard { .. } => Some(MatchedCase {
                condition: "true".to_string(),
                bindings: parent_bindings.clone(),
                binding_types: parent_binding_types.clone(),
            }),
            ast::Pattern::Alias { inner, name, .. } => {
                let mut matched = self.match_case_pattern(
                    inner,
                    value,
                    value_ty,
                    index,
                    parent_bindings,
                    parent_binding_types,
                )?;
                if name != "_" {
                    let (alias_value, alias_ty) = self.pattern_alias_value(inner, value, value_ty);
                    matched.bindings.insert(name.clone(), alias_value);
                    matched.binding_types.insert(name.clone(), alias_ty);
                }
                Some(matched)
            }
            ast::Pattern::Extract { inner, .. } => {
                let inner_ty = lifted_success_type(value_ty)?;
                let (case_type, accessor) = match value_ty {
                    ir::Type::Named { name, .. } if name == "Option" => {
                        ("lume.core.Option.Some<?>".to_string(), "value")
                    }
                    ir::Type::Named { name, .. } if name == "Result" => {
                        ("lume.core.Result.Ok<?, ?>".to_string(), "value")
                    }
                    ir::Type::Named { name, .. } if name == "Either" => {
                        ("lume.core.Either.Right<?, ?>".to_string(), "value")
                    }
                    _ => return None,
                };
                let extracted_local = format!("__extract{index}");
                let extracted = format!(
                    "(({}) {extracted_local}.{accessor}())",
                    self.names.value_type(&inner_ty),
                );
                let mut matched = self.match_case_pattern(
                    inner,
                    &extracted,
                    &inner_ty,
                    index,
                    parent_bindings,
                    parent_binding_types,
                )?;
                matched.condition = format!(
                    "{value} instanceof {case_type} {extracted_local} && ({})",
                    matched.condition
                );
                Some(matched)
            }
            ast::Pattern::Literal { value: literal, .. } => Some(MatchedCase {
                condition: format!(
                    "java.util.Objects.equals({value}, {})",
                    emit_pattern_literal(literal)?
                ),
                bindings: parent_bindings.clone(),
                binding_types: parent_binding_types.clone(),
            }),
            ast::Pattern::Type { name, target, .. } => {
                let target = type_ref_to_ir(target);
                let java_type = self.names.value_type(&target);
                let raw_java_type = java_type.split('<').next().unwrap_or(&java_type);
                let mut bindings = parent_bindings.clone();
                let mut binding_types = parent_binding_types.clone();
                if let Some(name) = name.as_ref().filter(|name| name.as_str() != "_") {
                    bindings.insert(name.clone(), format!("(({java_type}) {value})"));
                    binding_types.insert(name.clone(), target);
                }
                Some(MatchedCase {
                    condition: format!("{value} instanceof {raw_java_type}"),
                    bindings,
                    binding_types,
                })
            }
            ast::Pattern::Record { path, fields, .. } => {
                let case_name = path.last().map(String::as_str).or_else(|| match value_ty {
                    ir::Type::Named { name, .. } => Some(name.as_str()),
                    _ => None,
                })?;
                if let Some((owner, enum_case)) =
                    self.declared_enum_case_for_pattern(path, value_ty)
                {
                    self.enum_record_case_match(
                        owner,
                        enum_case,
                        fields,
                        value,
                        index,
                        parent_bindings,
                        parent_binding_types,
                    )
                } else if core_enum_case_owner(case_name).is_some() {
                    self.core_enum_record_case_match(
                        case_name,
                        fields,
                        value,
                        value_ty,
                        index,
                        parent_bindings,
                        parent_binding_types,
                    )
                } else {
                    self.declared_record_pattern_match(
                        case_name,
                        fields,
                        value,
                        index,
                        parent_bindings,
                        parent_binding_types,
                    )
                }
            }
            ast::Pattern::Constructor { path, args, .. } => {
                let case_name = path.last()?;
                if let Some((owner, enum_case)) =
                    self.declared_enum_case_for_pattern(path, value_ty)
                {
                    self.enum_case_match(
                        owner,
                        enum_case,
                        args,
                        value,
                        index,
                        parent_bindings,
                        parent_binding_types,
                    )
                } else if core_enum_case_owner(case_name).is_some() {
                    self.core_enum_case_match(
                        case_name,
                        args,
                        value,
                        value_ty,
                        index,
                        parent_bindings,
                        parent_binding_types,
                    )
                } else if args.is_empty()
                    && self
                        .bundle
                        .ir
                        .types
                        .iter()
                        .any(|ty| ty.name == *case_name && ty.kind == TypeKind::Object)
                {
                    Some(MatchedCase {
                        condition: format!(
                            "{value} instanceof {}",
                            self.names.named_type(case_name)
                        ),
                        bindings: parent_bindings.clone(),
                        binding_types: parent_binding_types.clone(),
                    })
                } else {
                    None
                }
            }
            ast::Pattern::Binding { name, .. }
                if self
                    .declared_enum_case_for_pattern(std::slice::from_ref(name), value_ty)
                    .is_some() =>
            {
                let (owner, enum_case) =
                    self.declared_enum_case_for_pattern(std::slice::from_ref(name), value_ty)?;
                self.enum_case_match(
                    owner,
                    enum_case,
                    &[],
                    value,
                    index,
                    parent_bindings,
                    parent_binding_types,
                )
            }
            ast::Pattern::Binding { name, .. } if core_enum_case_owner(name).is_some() => self
                .core_enum_case_match(
                    name,
                    &[],
                    value,
                    value_ty,
                    index,
                    parent_bindings,
                    parent_binding_types,
                ),
            ast::Pattern::Binding { name, .. }
                if self
                    .bundle
                    .ir
                    .types
                    .iter()
                    .any(|ty| ty.name == *name && ty.kind == TypeKind::Object) =>
            {
                Some(MatchedCase {
                    condition: format!("{value} instanceof {}", self.names.named_type(name)),
                    bindings: parent_bindings.clone(),
                    binding_types: parent_binding_types.clone(),
                })
            }
            ast::Pattern::Binding { name, .. } => {
                let mut bindings = parent_bindings.clone();
                let mut binding_types = parent_binding_types.clone();
                bindings.insert(name.clone(), value.to_string());
                binding_types.insert(name.clone(), value_ty.clone());
                Some(MatchedCase {
                    condition: "true".to_string(),
                    bindings,
                    binding_types,
                })
            }
            ast::Pattern::List { elements, rest, .. } => self.list_pattern_match(
                elements,
                rest.as_ref(),
                value,
                value_ty,
                index,
                parent_bindings,
                parent_binding_types,
            ),
            _ => None,
        }
    }

    fn pattern_alias_value(
        &self,
        pattern: &ast::Pattern,
        value: &str,
        value_ty: &ir::Type,
    ) -> (String, ir::Type) {
        let declared_path = match pattern {
            ast::Pattern::Record { path, .. } | ast::Pattern::Constructor { path, .. } => {
                Some(path.as_slice())
            }
            ast::Pattern::Binding { name, .. } => Some(std::slice::from_ref(name)),
            _ => None,
        };
        if let Some(path) = declared_path
            && let Some((owner, enum_case)) = self.declared_enum_case_for_pattern(path, value_ty)
        {
            let case_type = format!(
                "{}.{}{}",
                self.names.named_type(&owner.name),
                java_type_name(&enum_case.name),
                java_wildcard_type_args(owner.type_params.len())
            );
            let args = match value_ty {
                ir::Type::Named { name, args } if name == &owner.name => args.clone(),
                _ => vec![ir::Type::Unknown; owner.type_params.len()],
            };
            return (
                format!("(({case_type}) {value})"),
                ir::Type::Named {
                    name: format!("{}::{}", owner.name, enum_case.name),
                    args,
                },
            );
        }

        let named_pattern = match pattern {
            ast::Pattern::Record { path, .. } | ast::Pattern::Constructor { path, .. } => {
                path.last().map(String::as_str)
            }
            ast::Pattern::Binding { name, .. }
                if core_enum_case_owner(name).is_some() || self.enum_case(name).is_some() =>
            {
                Some(name.as_str())
            }
            _ => None,
        };

        if let Some(case_name) = named_pattern
            && self.enum_case(case_name).is_some()
        {
            let case_type = format!(
                "{}{}",
                java_type_name(case_name),
                java_wildcard_type_args(
                    self.owner.map(|owner| owner.type_params.len()).unwrap_or(0)
                )
            );
            return (format!("(({case_type}) {value})"), value_ty.clone());
        }

        if let Some(case_name) = named_pattern
            && let Some(owner) = core_enum_case_owner(case_name)
        {
            let arity = if owner == "Option" { 1 } else { 2 };
            let case_type = format!(
                "lume.core.{owner}.{}{}",
                java_type_name(case_name),
                java_wildcard_type_args(arity)
            );
            let args = match value_ty {
                ir::Type::Named { name, args } if name == owner => args.clone(),
                _ => vec![ir::Type::Unknown; arity],
            };
            return (
                format!("(({case_type}) {value})"),
                ir::Type::Named {
                    name: format!("{owner}::{case_name}"),
                    args,
                },
            );
        }

        if let Some(type_name) = named_pattern {
            if self.bundle.ir.types.iter().any(|ty| ty.name == type_name) {
                let java_type = self.names.named_type(type_name);
                return (
                    format!("(({java_type}) {value})"),
                    ir::Type::Named {
                        name: type_name.to_string(),
                        args: Vec::new(),
                    },
                );
            }
        }

        if let ast::Pattern::Type { target, .. } = pattern {
            let target = type_ref_to_ir(target);
            return (
                format!("(({}) {value})", self.names.value_type(&target)),
                target,
            );
        }

        (value.to_string(), value_ty.clone())
    }

    #[allow(clippy::too_many_arguments)]
    fn list_pattern_match(
        &self,
        elements: &[ast::Pattern],
        rest: Option<&ast::ListPatternRest>,
        value: &str,
        value_ty: &ir::Type,
        index: usize,
        parent_bindings: &HashMap<String, String>,
        parent_binding_types: &HashMap<String, ir::Type>,
    ) -> Option<MatchedCase> {
        let ir::Type::Named { name, args } = value_ty else {
            return None;
        };
        if name != "Vector" || args.len() != 1 {
            return None;
        }
        let element_ty = &args[0];
        let comparison = if rest.is_some() { ">=" } else { "==" };
        let mut conditions = vec![format!(
            "lume.core.LumeRuntime.listLen({value}) {comparison} {}L",
            elements.len()
        )];
        let mut bindings = parent_bindings.clone();
        let mut binding_types = parent_binding_types.clone();
        for (offset, pattern) in elements.iter().enumerate() {
            let item = format!(
                "(({}) lume.core.LumeRuntime.listGet({value}, {offset}L))",
                self.names.value_type(element_ty)
            );
            let matched = self.match_case_pattern(
                pattern,
                &item,
                element_ty,
                index + offset + 1,
                &bindings,
                &binding_types,
            )?;
            if matched.condition != "true" {
                conditions.push(format!("({})", matched.condition));
            }
            bindings = matched.bindings;
            binding_types = matched.binding_types;
        }
        if let Some(rest) = rest
            && rest.name != "_"
        {
            bindings.insert(
                rest.name.clone(),
                format!(
                    "((lume.core.LumeVector<{}>) lume.core.LumeRuntime.listSlice({value}, {}L))",
                    self.names.value_type(element_ty),
                    elements.len()
                ),
            );
            binding_types.insert(rest.name.clone(), value_ty.clone());
        }
        Some(MatchedCase {
            condition: conditions.join(" && "),
            bindings,
            binding_types,
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn declared_record_pattern_match(
        &self,
        type_name: &str,
        fields: &[ast::RecordPatternField],
        value: &str,
        index: usize,
        parent_bindings: &HashMap<String, String>,
        parent_binding_types: &HashMap<String, ir::Type>,
    ) -> Option<MatchedCase> {
        let ty = self
            .bundle
            .ir
            .types
            .iter()
            .find(|ty| ty.name == type_name)?;
        if !matches!(
            ty.kind,
            TypeKind::Class | TypeKind::Record | TypeKind::Object | TypeKind::Enum
        ) {
            return None;
        }
        let java_type = self.names.named_type(type_name);
        let case_local = format!("__case{index}");
        let needs_case_local = fields
            .iter()
            .any(|field| !matches!(field.pattern, ast::Pattern::Wildcard { .. }));
        let mut conditions = vec![if needs_case_local {
            format!("{value} instanceof {java_type} {case_local}")
        } else {
            format!("{value} instanceof {java_type}")
        }];
        let mut bindings = parent_bindings.clone();
        let mut binding_types = parent_binding_types.clone();
        for field_pattern in fields {
            let field = ty
                .fields
                .iter()
                .find(|field| field.name == field_pattern.name)?;
            let member = java_member_name(&field.name);
            let access = match ty.kind {
                TypeKind::Record => format!("{case_local}.{member}()"),
                TypeKind::Enum => format!(
                    "(({}) lume.core.LumeRuntime.patternField({}, {}))",
                    self.names.value_type(&field.ty),
                    case_local,
                    java_string_literal(&field.name)
                ),
                _ => format!("{case_local}.{member}"),
            };
            match &field_pattern.pattern {
                ast::Pattern::Wildcard { .. } => {}
                ast::Pattern::Binding { name, .. } => {
                    if name != "_" {
                        bindings.insert(name.clone(), access);
                        binding_types.insert(name.clone(), field.ty.clone());
                    }
                }
                ast::Pattern::Literal { value, .. } => conditions.push(format!(
                    "java.util.Objects.equals({access}, {})",
                    emit_pattern_literal(value)?
                )),
                pattern => {
                    let matched = self.match_case_pattern(
                        pattern,
                        &access,
                        &field.ty,
                        index + conditions.len(),
                        &bindings,
                        &binding_types,
                    )?;
                    if matched.condition != "true" {
                        conditions.push(format!("({})", matched.condition));
                    }
                    bindings = matched.bindings;
                    binding_types = matched.binding_types;
                }
            }
        }
        Some(MatchedCase {
            condition: conditions.join(" && "),
            bindings,
            binding_types,
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn core_enum_record_case_match(
        &self,
        case_name: &str,
        fields: &[ast::RecordPatternField],
        value: &str,
        value_ty: &ir::Type,
        index: usize,
        parent_bindings: &HashMap<String, String>,
        parent_binding_types: &HashMap<String, ir::Type>,
    ) -> Option<MatchedCase> {
        let field_defs = core_enum_case_fields(case_name, value_ty)?;
        let patterns = fields
            .iter()
            .map(|field| {
                field_defs
                    .iter()
                    .find(|(name, _)| name == &field.name)
                    .map(|(_, ty)| (&field.pattern, field.name.as_str(), ty.clone()))
            })
            .collect::<Option<Vec<_>>>()?;
        self.core_enum_case_pattern_match(
            case_name,
            &patterns,
            value,
            index,
            parent_bindings,
            parent_binding_types,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn core_enum_case_match(
        &self,
        case_name: &str,
        args: &[ast::Pattern],
        value: &str,
        value_ty: &ir::Type,
        index: usize,
        parent_bindings: &HashMap<String, String>,
        parent_binding_types: &HashMap<String, ir::Type>,
    ) -> Option<MatchedCase> {
        let fields = core_enum_case_fields(case_name, value_ty)?;
        if fields.len() != args.len() {
            return None;
        }
        let patterns = args
            .iter()
            .zip(fields)
            .map(|(pattern, (name, ty))| (pattern, name, ty))
            .collect::<Vec<_>>();
        self.core_enum_case_pattern_match(
            case_name,
            &patterns,
            value,
            index,
            parent_bindings,
            parent_binding_types,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn core_enum_case_pattern_match(
        &self,
        case_name: &str,
        patterns: &[(&ast::Pattern, &str, ir::Type)],
        value: &str,
        index: usize,
        parent_bindings: &HashMap<String, String>,
        parent_binding_types: &HashMap<String, ir::Type>,
    ) -> Option<MatchedCase> {
        let owner = core_enum_case_owner(case_name)?;
        let case_local = format!("__case{index}");
        let arity = match owner {
            "Option" => 1,
            "Result" | "Either" => 2,
            _ => return None,
        };
        let case_type = format!(
            "lume.core.{owner}.{}{}",
            java_type_name(case_name),
            java_wildcard_type_args(arity)
        );
        let needs_case_local = patterns
            .iter()
            .any(|(pattern, _, _)| !matches!(pattern, ast::Pattern::Wildcard { .. }));
        let condition = if needs_case_local {
            format!("{value} instanceof {case_type} {case_local}")
        } else {
            format!("{value} instanceof {case_type}")
        };
        let mut bindings = parent_bindings.clone();
        let mut binding_types = parent_binding_types.clone();
        for (pattern, field_name, field_ty) in patterns {
            match pattern {
                ast::Pattern::Wildcard { .. } => {}
                ast::Pattern::Binding { name, .. } => {
                    if name == "_" {
                        continue;
                    }
                    bindings.insert(
                        name.clone(),
                        format!(
                            "(({}) {case_local}.{}())",
                            self.names.value_type(field_ty),
                            java_member_name(field_name)
                        ),
                    );
                    binding_types.insert(name.clone(), field_ty.clone());
                }
                _ => return None,
            }
        }
        Some(MatchedCase {
            condition,
            bindings,
            binding_types,
        })
    }

    fn enum_record_case_match(
        &self,
        owner: &ir::TypeDef,
        enum_case: &ir::EnumCase,
        fields: &[ast::RecordPatternField],
        value: &str,
        index: usize,
        parent_bindings: &HashMap<String, String>,
        parent_binding_types: &HashMap<String, ir::Type>,
    ) -> Option<MatchedCase> {
        let java_case = java_type_name(&enum_case.name);
        let case_local = format!("__case{index}");
        let owner_prefix = if self.owner.is_some_and(|current| current.name == owner.name) {
            String::new()
        } else {
            format!("{}.", self.names.named_type(&owner.name))
        };
        let case_type = format!(
            "{owner_prefix}{java_case}{}",
            java_wildcard_type_args(owner.type_params.len())
        );
        let needs_case_local = fields
            .iter()
            .any(|field| matches!(field.pattern, ast::Pattern::Binding { .. }));
        let condition = if needs_case_local {
            format!("{value} instanceof {case_type} {case_local}")
        } else {
            format!("{value} instanceof {case_type}")
        };
        let mut bindings = parent_bindings.clone();
        let mut binding_types = parent_binding_types.clone();
        for field_pattern in fields {
            let field = enum_case
                .fields
                .iter()
                .find(|field| field.name == field_pattern.name)?;
            match &field_pattern.pattern {
                ast::Pattern::Wildcard { .. } => {}
                ast::Pattern::Binding { name, .. } => {
                    if name == "_" {
                        continue;
                    }
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
                _ => return None,
            }
        }

        Some(MatchedCase {
            condition,
            bindings,
            binding_types,
        })
    }

    fn enum_case_match(
        &self,
        owner: &ir::TypeDef,
        enum_case: &ir::EnumCase,
        args: &[ast::Pattern],
        value: &str,
        index: usize,
        parent_bindings: &HashMap<String, String>,
        parent_binding_types: &HashMap<String, ir::Type>,
    ) -> Option<MatchedCase> {
        if enum_case.fields.len() != args.len() {
            return None;
        }

        let java_case = java_type_name(&enum_case.name);
        let case_local = format!("__case{index}");
        let owner_prefix = if self.owner.is_some_and(|current| current.name == owner.name) {
            String::new()
        } else {
            format!("{}.", self.names.named_type(&owner.name))
        };
        let case_type = format!(
            "{owner_prefix}{java_case}{}",
            java_wildcard_type_args(owner.type_params.len())
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

    fn declared_enum_case_for_pattern<'b>(
        &'b self,
        path: &[String],
        value_ty: &ir::Type,
    ) -> Option<(&'b ir::TypeDef, &'b ir::EnumCase)> {
        let case_name = path.last()?;
        let owner_name = if path.len() > 1 {
            path.get(path.len() - 2).map(String::as_str)
        } else if let ir::Type::Named { name, .. } = value_ty {
            Some(name.as_str())
        } else {
            self.owner.map(|owner| owner.name.as_str())
        }?;
        let owner = self
            .bundle
            .ir
            .types
            .iter()
            .find(|ty| ty.name == owner_name)?;
        let enum_case = owner
            .enum_cases
            .iter()
            .find(|enum_case| enum_case.name == *case_name)?;
        Some((owner, enum_case))
    }

    fn enum_case(&self, name: &str) -> Option<&'a ir::EnumCase> {
        self.owner?.enum_cases.iter().find(|case| case.name == name)
    }

    fn emit_expr(&self, expr: &core::Expr, bindings: &HashMap<String, String>) -> Option<String> {
        match expr {
            core::Expr::Identifier { name, .. } if name == "this" => Some("this".to_string()),
            core::Expr::Identifier { name, .. } if self.enum_case(name).is_some() => {
                if self.enum_case(name)?.kind == TypeKind::Object {
                    Some(format!("{}.instance()", java_type_name(name)))
                } else {
                    Some(format!("new {}<>()", java_type_name(name)))
                }
            }
            core::Expr::Identifier { name, .. } if core_enum_case_owner(name).is_some() => {
                self.emit_core_enum_case(name, &[])
            }
            core::Expr::Identifier { name, .. }
                if self
                    .bundle
                    .ir
                    .types
                    .iter()
                    .filter(|owner| {
                        owner
                            .enum_cases
                            .iter()
                            .any(|case| case.name == *name && case.kind == TypeKind::Object)
                    })
                    .count()
                    == 1 =>
            {
                let owner = self.bundle.ir.types.iter().find(|owner| {
                    owner
                        .enum_cases
                        .iter()
                        .any(|case| case.name == *name && case.kind == TypeKind::Object)
                })?;
                JavaIrSupport::new(self.bundle, self.function, self.names)
                    .emit_enum_case_call(&owner.name, name, &[])
            }
            core::Expr::Identifier { name, .. }
                if self
                    .bundle
                    .ir
                    .types
                    .iter()
                    .any(|ty| ty.name == *name && ty.kind == TypeKind::Object) =>
            {
                Some(format!("{}.INSTANCE", self.names.named_type(name)))
            }
            core::Expr::Identifier { name, .. } => bindings
                .get(name)
                .cloned()
                .or_else(|| self.lazy_param_value_reference(name))
                .or_else(|| self.param_reference(name))
                .or_else(|| self.implicit_field_reference(name))
                .or_else(|| {
                    if std::env::var_os("LUME_JAVA_DEBUG_STUBS").is_some() {
                        let mut visible = bindings.keys().cloned().collect::<Vec<_>>();
                        visible.sort();
                        eprintln!(
                            "readable java cannot resolve identifier '{name}' in '{}'; bindings={visible:?}, params={:?}",
                            self.function.name,
                            self.function
                                .params
                                .iter()
                                .filter_map(|id| self.function.locals.get(id.0))
                                .map(|local| local.name.as_str())
                                .collect::<Vec<_>>()
                        );
                    }
                    None
                }),
            core::Expr::Member {
                receiver, name, ..
            } if matches!(
                receiver.as_ref(),
                core::Expr::Identifier { name: owner, .. }
                    if self.bundle.ir.types.iter().any(|ty| {
                        ty.name == *owner && ty.enum_cases.iter().any(|case| case.name == *name)
                    })
            ) => {
                let core::Expr::Identifier { name: owner, .. } = receiver.as_ref() else {
                    unreachable!();
                };
                let case = self
                    .bundle
                    .ir
                    .types
                    .iter()
                    .find(|ty| ty.name == *owner)?
                    .enum_cases
                    .iter()
                    .find(|case| case.name == *name)?;
                if !case.fields.is_empty() {
                    return None;
                }
                if case.kind == TypeKind::Object {
                    Some(format!(
                        "{}.{}.instance()",
                        self.names.named_type(owner),
                        java_type_name(name)
                    ))
                } else {
                    let generic = self
                        .bundle
                        .ir
                        .types
                        .iter()
                        .find(|ty| ty.name == *owner)
                        .is_some_and(|ty| !ty.type_params.is_empty())
                        .then_some("<>")
                        .unwrap_or("");
                    Some(format!(
                        "new {}.{}{generic}()",
                        self.names.named_type(owner),
                        java_type_name(name)
                    ))
                }
            }
            core::Expr::Bool { value, .. } => Some(value.to_string()),
            core::Expr::Integer { raw, .. } => Some(format!("{raw}L")),
            core::Expr::Float { raw, .. } => Some(raw.clone()),
            core::Expr::String { raw, .. } => {
                Some(java_string_literal(&decode_lume_string_literal(raw)))
            }
            core::Expr::Unit { .. } => Some("lume.core.LumeUnit.INSTANCE".to_string()),
            core::Expr::TupleLiteral { items, .. } if (2..=8).contains(&items.len()) => {
                let items = items
                    .iter()
                    .map(|item| self.emit_expr(item, bindings))
                    .collect::<Option<Vec<_>>>()?;
                Some(format!(
                    "new lume.core.Tuple{}<>({})",
                    items.len(),
                    items.join(", ")
                ))
            }
            core::Expr::ListLiteral { items, .. }
                if matches!(
                    self.expr_type(expr, bindings),
                    Some(ir::Type::Named { name, args }) if name == "Map" && args.len() == 2
                ) =>
            {
                let parts = items
                    .iter()
                    .map(|item| match item {
                        core::Expr::Spread { value, .. } => self.emit_expr(value, bindings),
                        core::Expr::Binary {
                            left,
                            op: ast::BinaryOp::Colon,
                            right,
                            ..
                        } => Some(format!(
                            "new lume.core.Tuple2<>({}, {})",
                            self.emit_expr(left, bindings)?,
                            self.emit_expr(right, bindings)?
                        )),
                        _ => None,
                    })
                    .collect::<Option<Vec<_>>>()?;
                if parts.is_empty() {
                    Some("lume.core.LumeMap.empty()".to_string())
                } else {
                    Some(format!("lume.core.LumeMap.fromParts({})", parts.join(", ")))
                }
            }
            core::Expr::ListLiteral { items, .. } => {
                if items
                    .iter()
                    .any(|item| matches!(item, core::Expr::Spread { .. }))
                {
                    let mut out = "lume.core.LumeVector.empty()".to_string();
                    for item in items {
                        match item {
                            core::Expr::Spread { value, .. } => {
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
            core::Expr::Spread { value, .. } => self.emit_expr(value, bindings),
            core::Expr::Member { receiver, name, .. } if name == "runtimeType" => Some(format!(
                "lume.core.LumeRuntime.runtimeTypeOf({})",
                self.emit_expr(receiver, bindings)?
            )),
            core::Expr::Member { receiver, name, .. } if matches!(receiver.as_ref(), core::Expr::Identifier { name, .. } if name == "this") =>
            {
                let receiver = self.emit_expr(receiver, bindings)?;
                let member = java_member_name(name);
                if self
                    .owner
                    .is_some_and(|owner| owner.kind == TypeKind::Record)
                {
                    Some(format!("{receiver}.{member}()"))
                } else {
                    Some(format!("{receiver}.{member}"))
                }
            }
            core::Expr::Member { receiver, name, .. } => {
                if let core::Expr::Identifier {
                    name: owner_name, ..
                } = receiver.as_ref()
                {
                    let emitter = JavaIrSupport::new(self.bundle, self.function, self.names);
                    if emitter.enum_case(owner_name, name).is_some() {
                        return emitter.emit_enum_case_call(owner_name, name, &[]);
                    }
                }
                if matches!(
                    self.expr_type(expr, bindings),
                    Some(ir::Type::Function { .. })
                ) {
                    return None;
                }
                let receiver_expr = self.emit_expr(receiver, bindings)?;
                let member = java_member_name(name);
                let receiver_ty = self.expr_type(receiver, bindings)?;
                match receiver_ty {
                    ir::Type::Tuple(_) => {
                        let accessor = tuple_accessor_name(name)?;
                        Some(format!("{receiver_expr}.{accessor}()"))
                    }
                    ir::Type::Named {
                        name: type_name, ..
                    } if enum_case_view_parts(&type_name).is_some()
                        || is_core_accessor_backed_type(&type_name)
                        || self.bundle.ir.types.iter().any(|ty| {
                            ty.name == type_name
                                && (ty.kind == TypeKind::Record
                                    || ((ty.kind == TypeKind::Interface
                                        || is_anonymous_object_type(ty))
                                        && ty.fields.iter().any(|field| field.name == *name)))
                        }) =>
                    {
                        Some(format!("{receiver_expr}.{member}()"))
                    }
                    ir::Type::Named { .. } => Some(format!("{receiver_expr}.{member}")),
                    _ => None,
                }
            }
            core::Expr::Index {
                receiver, index, ..
            } => {
                let receiver_expr = self.emit_expr(receiver, bindings)?;
                let index_expr = self.emit_expr(index, bindings)?;
                match self.expr_type(receiver, bindings)? {
                    ir::Type::Named { name, args }
                        if matches!(name.as_str(), "Vector" | "Array") && args.len() == 1 =>
                    {
                        let result_ty = self
                            .expr_type(expr, bindings)
                            .unwrap_or_else(|| args[0].clone());
                        Some(format!(
                            "(({}) lume.core.LumeRuntime.indexValue({receiver_expr}, {index_expr}))",
                            self.names.value_type(&result_ty)
                        ))
                    }
                    ir::Type::Named { name, args } if name == "Map" && args.len() == 2 => {
                        Some(format!("{receiver_expr}.get({index_expr})"))
                    }
                    _ => None,
                }
            }
            core::Expr::Is { left, target, .. } => {
                let value = self.emit_expr(left, bindings)?;
                let target = type_ref_to_ir(target);
                if matches!(target, ir::Type::Never) {
                    return Some("false".to_string());
                }
                if matches!(target, ir::Type::Unknown) || is_named_builtin(&target, "Any") {
                    return Some(format!("{value} != null"));
                }
                if matches!(target, ir::Type::Record(_)) {
                    return None;
                }
                let erased = match target {
                    ir::Type::Named { name, .. } => ir::Type::Named {
                        name,
                        args: Vec::new(),
                    },
                    ir::Type::Tuple(items) => ir::Type::Tuple(vec![ir::Type::Unknown; items.len()]),
                    ir::Type::Function { params, .. } => ir::Type::Function {
                        params: vec![ir::Type::Unknown; params.len()],
                        ret: Box::new(ir::Type::Unknown),
                    },
                    other => other,
                };
                let java_type = self.names.value_type(&erased);
                let raw_java_type = java_type.split('<').next().unwrap_or(&java_type);
                Some(format!("{value} instanceof {raw_java_type}"))
            }
            core::Expr::TypeOf { ty, .. } => {
                if let TypeRef::Named { name, args, .. } = ty
                    && args.is_empty()
                    && self.function.reified_type_params.contains(name)
                {
                    return self
                        .function
                        .params
                        .iter()
                        .filter_map(|param| self.function.locals.get(param.0))
                        .find(|local| local.name == format!("__type_{name}"))
                        .map(java_local_name);
                }
                Some(type_value_expr(&type_ref_to_ir(ty), self.names))
            }
            core::Expr::Lambda { params, body, span }
                if params.iter().all(|param| param.destructure.is_none()) =>
            {
                let mut lambda_bindings = bindings.clone();
                for param in &self.function.params {
                    if let Some(local) = self.function.locals.get(param.0) {
                        lambda_bindings
                            .entry(local.name.clone())
                            .or_insert_with(|| java_local_name(local));
                    }
                }
                let java_params = params
                    .iter()
                    .enumerate()
                    .map(|(index, param)| {
                        let name = if param.name == "_" {
                            format!("__ignored{index}")
                        } else {
                            format!("{}_arg{index}", java_member_name(&param.name))
                        };
                        if param.name != "_" {
                            lambda_bindings.insert(param.name.clone(), name.clone());
                        }
                        name
                    })
                    .collect::<Vec<_>>();
                if let core::Expr::Block { body, .. } = body.as_ref() {
                    let lambda_function = self.bundle.ir.functions.iter().find(|function| {
                        matches!(function.kind, FunctionKind::Lambda)
                            && function.span == Some(*span)
                            && function.params.len() == params.len()
                    })?;
                    let lambda_emitter = SourceBodyEmitter {
                        bundle: self.bundle,
                        function: lambda_function,
                        names: self.names,
                        owner: self.owner,
                    };
                    let mut lambda_types = HashMap::new();
                    for name in lambda_bindings.keys() {
                        if let Some(local) = self
                            .function
                            .locals
                            .iter()
                            .find(|local| local.name == *name)
                        {
                            lambda_types.insert(name.clone(), local.ty.clone());
                        }
                    }
                    for (index, param) in params.iter().enumerate() {
                        if param.name == "_" {
                            continue;
                        }
                        if let Some(local) = lambda_function
                            .params
                            .get(index)
                            .and_then(|param| lambda_function.locals.get(param.0))
                        {
                            lambda_types.insert(param.name.clone(), local.ty.clone());
                        }
                    }
                    let body = lambda_emitter.emit_inline_block(
                        body,
                        lambda_bindings,
                        lambda_types,
                        true,
                    )?;
                    Some(format!("({}) -> {body}", java_params.join(", ")))
                } else {
                    let body = self.emit_expr(body, &lambda_bindings)?;
                    Some(format!("({}) -> {body}", java_params.join(", ")))
                }
            }
            core::Expr::AnonymousObject { .. } => {
                let ty = self.expr_type(expr, bindings).or_else(|| {
                    if std::env::var_os("LUME_JAVA_DEBUG_STUBS").is_some() {
                        eprintln!(
                            "readable java cannot determine anonymous object type in '{}' at {:?}",
                            self.function.name,
                            expr.span()
                        );
                    }
                    None
                })?;
                let lowered = self.function.blocks.iter().find_map(|block| {
                    block.statements.iter().find_map(|statement| {
                        let ir::StatementKind::Assign {
                            value:
                                ir::RValue::AnonymousObject {
                                    ty: object_ty,
                                    fields,
                                    methods,
                                },
                            ..
                        } = &statement.kind
                        else {
                            return None;
                        };
                        (object_ty == &ty).then_some((object_ty, fields, methods))
                    })
                }).or_else(|| {
                    if std::env::var_os("LUME_JAVA_DEBUG_STUBS").is_some() {
                        eprintln!(
                            "readable java cannot find lowered anonymous object in '{}': type={ty:?}",
                            self.function.name
                        );
                    }
                    None
                })?;
                JavaIrSupport::new(self.bundle, self.function, self.names)
                    .emit_anonymous_object(lowered.0, lowered.1, lowered.2, None)
                    .or_else(|| {
                        if std::env::var_os("LUME_JAVA_DEBUG_STUBS").is_some() {
                            eprintln!(
                                "readable java cannot emit lowered anonymous object in '{}'",
                                self.function.name
                            );
                        }
                        None
                    })
            }
            core::Expr::AnonymousInterface { interfaces, .. } => {
                let interface_types = interfaces.iter().map(type_ref_to_ir).collect::<Vec<_>>();
                let lowered = self.function.blocks.iter().find_map(|block| {
                    block.statements.iter().find_map(|statement| {
                        let ir::StatementKind::Assign {
                            value:
                                ir::RValue::AnonymousInterface {
                                    interfaces,
                                    methods,
                                },
                            ..
                        } = &statement.kind
                        else {
                            return None;
                        };
                        (interfaces == &interface_types).then_some((interfaces, methods))
                    })
                })?;
                JavaIrSupport::new(self.bundle, self.function, self.names)
                    .emit_anonymous_interface(lowered.0, lowered.1)
            }
            core::Expr::ExtractOr {
                value,
                fallback,
                span,
            } => {
                let source_ty = self.expr_type(value, bindings)?;
                let success_ty = lifted_success_type(&source_ty)?;
                let (case_type, accessor) = match &source_ty {
                    ir::Type::Named { name, .. } if name == "Option" => {
                        ("lume.core.Option.Some<?>".to_string(), "value")
                    }
                    ir::Type::Named { name, .. } if name == "Result" => {
                        ("lume.core.Result.Ok<?, ?>".to_string(), "value")
                    }
                    ir::Type::Named { name, .. } if name == "Either" => {
                        ("lume.core.Either.Right<?, ?>".to_string(), "value")
                    }
                    _ => return None,
                };
                let source = self.emit_expr(value, bindings)?;
                let fallback = self.emit_expr_against(fallback, bindings, &success_ty)?;
                let local = format!("__extractOr{}", span.start);
                Some(format!(
                    "({source} instanceof {case_type} {local} ? (({}) {local}.{accessor}()) : {fallback})",
                    self.names.value_type(&success_ty)
                ))
            }
            core::Expr::Unary {
                op,
                expr: operand,
                span,
            } => {
                let value = self.emit_expr(operand, bindings)?;
                match op {
                    ast::UnaryOp::Neg => Some(format!("(-{value})")),
                    ast::UnaryOp::Not => Some(format!("(!({value}))")),
                    ast::UnaryOp::UnsafeExtract => {
                        let result_ty = self
                            .bundle
                            .ir
                            .source_exprs
                            .iter()
                            .find(|source| {
                                source.function == self.function.id && source.span == *span
                            })?
                            .ty
                            .clone();
                        Some(format!(
                            "(({}) lume.core.LumeRuntime.extractSuccessValue({value}))",
                            self.names.value_type(&result_ty)
                        ))
                    }
                }
            }
            core::Expr::Binary {
                left, op, right, ..
            } => match op {
                ast::BinaryOp::Eq => self.emit_equality(left, right, bindings, false),
                ast::BinaryOp::NotEq => self.emit_equality(left, right, bindings, true),
                ast::BinaryOp::IdentityEq => Some(format!(
                    "({} == {})",
                    self.emit_expr(left, bindings)?,
                    self.emit_expr(right, bindings)?
                )),
                ast::BinaryOp::IdentityNotEq => Some(format!(
                    "({} != {})",
                    self.emit_expr(left, bindings)?,
                    self.emit_expr(right, bindings)?
                )),
                ast::BinaryOp::Colon => None,
                _ => {
                    let left = self.emit_expr(left, bindings)?;
                    let right = self.emit_expr(right, bindings)?;
                    let operator = match op {
                        ast::BinaryOp::Or => "||",
                        ast::BinaryOp::And => "&&",
                        ast::BinaryOp::Less => "<",
                        ast::BinaryOp::LessEq => "<=",
                        ast::BinaryOp::Greater => ">",
                        ast::BinaryOp::GreaterEq => ">=",
                        ast::BinaryOp::Add => "+",
                        ast::BinaryOp::Sub => "-",
                        ast::BinaryOp::Mul => "*",
                        ast::BinaryOp::Div => "/",
                        ast::BinaryOp::Mod => "%",
                        ast::BinaryOp::Colon
                        | ast::BinaryOp::Eq
                        | ast::BinaryOp::NotEq
                        | ast::BinaryOp::IdentityEq
                        | ast::BinaryOp::IdentityNotEq => unreachable!(),
                    };
                    Some(format!("({left} {operator} {right})"))
                }
            },
            core::Expr::Match {
                partial: false,
                value,
                cases,
                span,
            } => self.emit_match_expression(value, cases, *span, bindings),
            core::Expr::Call {
                callee, args, span, ..
            } => {
                let emitted = self.emit_call(callee, args, *span, bindings);
                if emitted.is_none() && std::env::var_os("LUME_JAVA_DEBUG_STUBS").is_some() {
                    eprintln!(
                        "readable java cannot emit call in '{}': {:?}",
                        self.function.name, callee
                    );
                }
                emitted
            }
            _ => None,
        }
    }

    fn emit_match_expression(
        &self,
        value: &core::Expr,
        cases: &[core::MatchCase],
        span: crate::source::Span,
        bindings: &HashMap<String, String>,
    ) -> Option<String> {
        let value_ty = self.expr_type(value, bindings).or_else(|| {
            if std::env::var_os("LUME_JAVA_DEBUG_STUBS").is_some() {
                eprintln!(
                    "readable java cannot infer match value type in '{}': {value:?}",
                    self.function.name
                );
            }
            None
        })?;
        let result_ty = self
            .source_expr_type(span)
            .or_else(|| {
                cases.iter().find_map(|case| match &case.body {
                    core::MatchCaseBody::Expr(expr) => self.source_expr_type(expr.span()),
                    core::MatchCaseBody::Block(_) => None,
                })
            })
            .or_else(|| {
                if std::env::var_os("LUME_JAVA_DEBUG_STUBS").is_some() {
                    eprintln!(
                        "readable java cannot infer match result type in '{}': span={span:?}",
                        self.function.name
                    );
                }
                None
            })?;
        let match_local = format!("__match{}", span.start);
        let mut out = format!(
            "((java.util.function.Supplier<{}>) () -> {{\n            var {match_local} = {};\n",
            self.names.value_type(&result_ty),
            self.emit_expr(value, bindings)?
        );
        for (index, case) in cases.iter().enumerate() {
            let matched = self
                .match_case_pattern(
                    &case.pattern,
                    &match_local,
                    &value_ty,
                    span.start + index,
                    bindings,
                    &HashMap::new(),
                )
                .or_else(|| {
                    if std::env::var_os("LUME_JAVA_DEBUG_STUBS").is_some() {
                        eprintln!(
                            "readable java cannot emit match-expression pattern in '{}': {:?}",
                            self.function.name, case.pattern
                        );
                    }
                    None
                })?;
            let condition = match &case.guard {
                Some(guard) => format!(
                    "({}) && ({})",
                    matched.condition,
                    self.emit_expr(guard, &matched.bindings)?
                ),
                None => matched.condition,
            };
            let core::MatchCaseBody::Expr(body) = &case.body else {
                return None;
            };
            out.push_str("            if (");
            out.push_str(&condition);
            out.push_str(") {\n                return ");
            out.push_str(
                &self
                    .emit_expr_against(body, &matched.bindings, &result_ty)
                    .or_else(|| {
                        if std::env::var_os("LUME_JAVA_DEBUG_STUBS").is_some() {
                            eprintln!(
                                "readable java cannot emit match-expression body in '{}': {body:?}",
                                self.function.name
                            );
                        }
                        None
                    })?,
            );
            out.push_str(";\n            }\n");
        }
        out.push_str(
            "            throw new IllegalStateException(\"non-exhaustive Lume match\");\n        }).get()",
        );
        Some(out)
    }

    fn implicit_field_reference(&self, name: &str) -> Option<String> {
        let owner = self.owner?;
        owner.fields.iter().find(|field| field.name == name)?;
        let member = java_member_name(name);
        Some(if matches!(owner.kind, TypeKind::Record | TypeKind::Enum) {
            format!("this.{member}()")
        } else {
            format!("this.{member}")
        })
    }

    fn emit_equality(
        &self,
        left: &core::Expr,
        right: &core::Expr,
        bindings: &HashMap<String, String>,
        negated: bool,
    ) -> Option<String> {
        let left_expr = self.emit_expr(left, bindings)?;
        let mut right_expr = self.emit_expr(right, bindings)?;
        if let (Some(left_ty), Some(right_ty)) = (
            self.expr_type(left, bindings),
            self.expr_type(right, bindings),
        ) && left_ty != right_ty
            && let Some(projected) =
                self.emit_shape_equality_projection(&left_ty, right, &right_ty, bindings)
        {
            right_expr = projected;
        }
        let equality = format!("java.util.Objects.equals({left_expr}, {right_expr})");
        Some(if negated {
            format!("!{equality}")
        } else {
            equality
        })
    }

    fn emit_shape_equality_projection(
        &self,
        target: &ir::Type,
        source: &core::Expr,
        source_ty: &ir::Type,
        bindings: &HashMap<String, String>,
    ) -> Option<String> {
        if !matches!(source, core::Expr::Identifier { .. }) {
            return None;
        }
        let ir::Type::Named {
            name: target_name,
            args: target_args,
        } = target
        else {
            return None;
        };
        let ir::Type::Named {
            name: source_name,
            args: source_args,
        } = source_ty
        else {
            return None;
        };
        let target_def = self
            .bundle
            .ir
            .types
            .iter()
            .find(|ty| ty.name == *target_name && ty.kind == TypeKind::Record)?;
        let source_def = self
            .bundle
            .ir
            .types
            .iter()
            .find(|ty| ty.name == *source_name && ty.kind == TypeKind::Record)?;
        let target_subst = target_def
            .type_params
            .iter()
            .cloned()
            .zip(target_args.iter().cloned())
            .collect::<HashMap<_, _>>();
        let source_subst = source_def
            .type_params
            .iter()
            .cloned()
            .zip(source_args.iter().cloned())
            .collect::<HashMap<_, _>>();
        let source_fields = source_def
            .fields
            .iter()
            .filter(|field| field.visibility != Visibility::Hidden)
            .map(|field| {
                (
                    field.name.as_str(),
                    substitute_java_emit_type(&field.ty, &source_subst),
                )
            })
            .collect::<HashMap<_, _>>();
        let target_fields = target_def
            .fields
            .iter()
            .filter(|field| field.visibility != Visibility::Hidden)
            .collect::<Vec<_>>();
        if target_fields.len() != source_fields.len()
            || target_fields.iter().any(|field| {
                source_fields.get(field.name.as_str())
                    != Some(&substitute_java_emit_type(&field.ty, &target_subst))
            })
        {
            return None;
        }

        let source_expr = self.emit_expr(source, bindings)?;
        let args = target_fields
            .iter()
            .map(|field| format!("{source_expr}.{}()", java_member_name(&field.name)))
            .collect::<Vec<_>>()
            .join(", ");
        let generic = (!target_args.is_empty()).then_some("<>").unwrap_or("");
        Some(format!(
            "new {}{generic}({args})",
            self.names.named_type(target_name)
        ))
    }

    fn emit_expr_against(
        &self,
        expr: &core::Expr,
        bindings: &HashMap<String, String>,
        expected: &ir::Type,
    ) -> Option<String> {
        match expr {
            core::Expr::Call { callee, args, .. } if matches!(callee.as_ref(), core::Expr::Identifier { name, .. } if core_enum_case_owner(name).is_some()) =>
            {
                let core::Expr::Identifier { name, .. } = callee.as_ref() else {
                    unreachable!()
                };
                let payload_ty = core_enum_case_payload_type(name, expected);
                let emitted = args
                    .iter()
                    .enumerate()
                    .map(|(index, arg)| {
                        if index == 0
                            && let Some(payload_ty) = payload_ty.as_ref()
                        {
                            self.emit_expr_against(&arg.value, bindings, payload_ty)
                        } else {
                            self.emit_call_arg(arg, bindings)
                        }
                    })
                    .collect::<Option<Vec<_>>>()?;
                self.emit_core_enum_case(name, &emitted)
            }
            core::Expr::ListLiteral { items, .. }
                if items.is_empty()
                    && matches!(
                        expected,
                        ir::Type::Named { name, args } if name == "Map" && args.len() == 2
                    ) =>
            {
                Some("lume.core.LumeMap.empty()".to_string())
            }
            core::Expr::RecordLiteral { fields, values, .. } => {
                self.emit_record_literal_against(fields, values, bindings, expected)
            }
            _ => self.emit_expr(expr, bindings),
        }
    }

    fn emit_record_literal_against(
        &self,
        fields: &[core::CallArg],
        values: &[core::Expr],
        bindings: &HashMap<String, String>,
        expected: &ir::Type,
    ) -> Option<String> {
        let ir::Type::Named { name, args } = expected else {
            if std::env::var_os("LUME_JAVA_DEBUG_STUBS").is_some() {
                eprintln!(
                    "readable java cannot construct record literal against non-named type: {expected:?}"
                );
            }
            return None;
        };
        let type_def = self
            .bundle
            .ir
            .types
            .iter()
            .find(|ty| ty.name == *name && ty.kind == TypeKind::Record)?;
        let substitution = type_def
            .type_params
            .iter()
            .cloned()
            .zip(args.iter().cloned())
            .collect::<HashMap<_, _>>();

        let mut constructor_args = Vec::with_capacity(type_def.fields.len());
        if fields.is_empty() && !values.is_empty() {
            if values.len() != type_def.fields.len() {
                return None;
            }
            for (field, value) in type_def.fields.iter().zip(values) {
                let field_ty = substitute_java_emit_type(&field.ty, &substitution);
                constructor_args.push(self.emit_expr_against(value, bindings, &field_ty)?);
            }
        } else {
            if fields.iter().any(|field| field.name.is_none()) {
                return None;
            }
            for field in &type_def.fields {
                let field_ty = substitute_java_emit_type(&field.ty, &substitution);
                if let Some(value) = fields
                    .iter()
                    .find(|value| value.name.as_deref() == Some(field.name.as_str()))
                {
                    constructor_args.push(self.emit_expr_against(
                        &value.value,
                        bindings,
                        &field_ty,
                    )?);
                } else if let Some(initializer) = &field.initializer {
                    constructor_args.push(java_constant(initializer));
                } else if field.has_initializer {
                    constructor_args.push(java_default_value(&field_ty));
                } else {
                    return None;
                }
            }
        }

        let generic = (!args.is_empty()).then_some("<>").unwrap_or("");
        Some(format!(
            "new {}{generic}({})",
            self.names.named_type(name),
            constructor_args.join(", ")
        ))
    }

    fn emit_call(
        &self,
        callee: &core::Expr,
        args: &[core::CallArg],
        span: crate::source::Span,
        bindings: &HashMap<String, String>,
    ) -> Option<String> {
        match callee {
            core::Expr::Index { receiver, .. }
                if self.source_call(span).is_some_and(|call| {
                    matches!(
                        call.callee,
                        ir::Callee::Direct(_)
                            | ir::Callee::Named { .. }
                            | ir::Callee::Method { .. }
                    )
                }) =>
            {
                self.emit_call(receiver, args, span, bindings)
            }
            core::Expr::Identifier { name, .. } if name == "Any" && args.len() == 1 => {
                self.emit_call_arg(&args[0], bindings)
            }
            core::Expr::Identifier { name, .. } if name == "panic" => {
                let message = match args.first() {
                    Some(arg) => self.emit_expr(&arg.value, bindings)?,
                    None => java_string_literal("panic"),
                };
                Some(format!("lume.core.LumePanic.panic({message})"))
            }
            core::Expr::Identifier { name, .. } if self.enum_case(name).is_some() => {
                let args = args
                    .iter()
                    .map(|arg| self.emit_call_arg(arg, bindings))
                    .collect::<Option<Vec<_>>>()?;
                if self.enum_case(name)?.kind == TypeKind::Object {
                    return args
                        .is_empty()
                        .then(|| format!("{}.instance()", java_type_name(name)));
                }
                Some(format!(
                    "new {}<>({})",
                    java_type_name(name),
                    args.join(", ")
                ))
            }
            core::Expr::Identifier { name, .. } if core_enum_case_owner(name).is_some() => {
                let source_ty = self.source_expr_type(span);
                let payload_ty = source_ty
                    .as_ref()
                    .and_then(|ty| core_enum_case_payload_type(name, ty));
                if std::env::var_os("LUME_JAVA_DEBUG_STUBS").is_some() && payload_ty.is_none() {
                    eprintln!(
                        "readable java has no payload type for core case '{name}' in '{}': {source_ty:?}",
                        self.function.name
                    );
                }
                let args = args
                    .iter()
                    .enumerate()
                    .map(|(index, arg)| {
                        if index == 0
                            && let Some(expected) = payload_ty.as_ref()
                        {
                            self.emit_expr_against(&arg.value, bindings, expected)
                        } else {
                            self.emit_call_arg(arg, bindings)
                        }
                    })
                    .collect::<Option<Vec<_>>>()?;
                self.emit_core_enum_case(name, &args)
            }
            core::Expr::Identifier { name, .. } if self.function_param(name).is_some() => {
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
            core::Expr::Identifier { name, .. } if bindings.contains_key(name) => {
                let args = args
                    .iter()
                    .map(|arg| self.emit_call_arg(arg, bindings))
                    .collect::<Option<Vec<_>>>()?;
                emit_functional_call(bindings.get(name)?, &args)
            }
            core::Expr::Identifier { name, .. }
                if self.owner.is_some_and(|owner| {
                    owner.methods.iter().any(|method| {
                        self.bundle.ir.function(*method).is_some_and(|function| {
                            function.name == *name
                                && param_specs_accept_arg_len(
                                    &source_function_param_specs(function),
                                    args.len(),
                                )
                        })
                    })
                }) =>
            {
                let target = self.owner?.methods.iter().find_map(|method| {
                    let function = self.bundle.ir.function(*method)?;
                    (function.name == *name
                        && param_specs_accept_arg_len(
                            &source_function_param_specs(function),
                            args.len(),
                        ))
                    .then_some(function)
                })?;
                let emitted = self.emit_source_args_for_param_specs(
                    &args.iter().collect::<Vec<_>>(),
                    &source_function_param_specs(target),
                    bindings,
                )?;
                Some(format!(
                    "this.{}({})",
                    java_member_name(name),
                    emitted.join(", ")
                ))
            }
            core::Expr::Identifier { name, .. } => {
                let call = self.source_call(span)?;
                if let ir::Callee::Intrinsic(intrinsic) = &call.callee {
                    return self.emit_source_intrinsic_call(intrinsic, args, call, bindings);
                }
                if let ir::Callee::Named { path } = &call.callee {
                    return self.emit_source_named_call(path, args, call, bindings);
                }
                if let ir::Callee::Method { method, .. } = &call.callee {
                    let target = self.bundle.ir.function(call.function)?;
                    let ordered_args = self.ordered_source_args(args, call)?;
                    let param_specs = source_function_param_specs(target);
                    let mut emitted = self.emit_source_args_for_param_specs(
                        &ordered_args,
                        &param_specs,
                        bindings,
                    )?;
                    emitted.extend(
                        self.emit_source_reified_evidence(call, target.reified_type_params.len())?,
                    );
                    return Some(format!(
                        "this.{}({})",
                        java_member_name(method),
                        emitted.join(", ")
                    ));
                }
                let ir::Callee::Direct(target) = call.callee else {
                    return None;
                };
                let target = self.bundle.ir.function(target)?;
                if target.name != *name {
                    return None;
                }
                let ordered_args = self.ordered_source_args(args, call)?;
                let param_specs = source_function_param_specs(target);
                let mut args =
                    self.emit_source_args_for_param_specs(&ordered_args, &param_specs, bindings)?;
                args.extend(
                    self.emit_source_reified_evidence(call, target.reified_type_params.len())?,
                );
                let method = java_member_name(&target.name);
                match target.kind {
                    FunctionKind::Method { .. } => {
                        Some(format!("this.{method}({})", args.join(", ")))
                    }
                    FunctionKind::TopLevel
                        if matches!(self.function.kind, FunctionKind::TopLevel) =>
                    {
                        Some(format!("{method}({})", args.join(", ")))
                    }
                    FunctionKind::TopLevel => Some(format!(
                        "{}.{}({})",
                        module_class_name(self.bundle),
                        method,
                        args.join(", ")
                    )),
                    FunctionKind::Local { .. } | FunctionKind::Lambda | FunctionKind::Synthetic => {
                        None
                    }
                }
            }
            core::Expr::Member { receiver, name, .. }
                if matches!(receiver.as_ref(), core::Expr::Identifier { name, .. } if name == "Array")
                    && matches!(
                        name.as_str(),
                        "ofInt" | "ofFloat" | "ofBool" | "ofStr" | "ofRune" | "fill"
                    ) =>
            {
                let args = args
                    .iter()
                    .map(|arg| self.emit_call_arg(arg, bindings))
                    .collect::<Option<Vec<_>>>()?;
                Some(format!(
                    "lume.core.LumeArray.{}({})",
                    java_member_name(name),
                    args.join(", ")
                ))
            }
            core::Expr::Member { receiver, name, .. }
                if name == "iterator"
                    && args.is_empty()
                    && matches!(
                        receiver.as_ref(),
                        core::Expr::ListLiteral { items, .. } if items.is_empty()
                    ) =>
            {
                Some("lume.core.LumeIterator.from(lume.core.LumeVector.of())".to_string())
            }
            core::Expr::Member { receiver, name, .. }
                if name == "iterator"
                    && args.is_empty()
                    && matches!(
                        receiver.as_ref(),
                        core::Expr::Call {
                            callee,
                            args,
                            style: core::CallStyle::Paren,
                            ..
                        } if args.is_empty()
                            && matches!(
                                callee.as_ref(),
                                core::Expr::Identifier { name, .. } if name == "Vector"
                            )
                    ) =>
            {
                Some("lume.core.LumeIterator.from(lume.core.LumeVector.of())".to_string())
            }
            core::Expr::Member { receiver, name, .. }
                if name == "parse"
                    && matches!(
                        receiver.as_ref(),
                        core::Expr::Identifier { name, .. } if name == "Int" || name == "Float"
                    ) =>
            {
                let core::Expr::Identifier { name: owner, .. } = receiver.as_ref() else {
                    unreachable!()
                };
                let [arg] = args else {
                    return None;
                };
                let value = self.emit_call_arg(arg, bindings)?;
                let method = if owner == "Int" {
                    "parseInt"
                } else {
                    "parseFloat"
                };
                Some(format!("lume.core.LumeRuntime.{method}({value})"))
            }
            core::Expr::Member { receiver, name, .. } if name == "toStr" && args.is_empty() => {
                Some(format!(
                    "String.valueOf({})",
                    self.emit_expr(receiver, bindings)?
                ))
            }
            core::Expr::Member { receiver, name, .. } if name == "equals" && args.len() == 1 => {
                self.emit_equality(receiver, &args[0].value, bindings, false)
            }
            core::Expr::Member { receiver, name, .. } if name == "sameValue" && args.len() == 1 => {
                Some(format!(
                    "lume.core.LumeRuntime.sameValue({}, {})",
                    self.emit_expr(receiver, bindings)?,
                    self.emit_call_arg(&args[0], bindings)?
                ))
            }
            core::Expr::Member { receiver, name, .. } if name == "hash" && args.is_empty() => {
                Some(format!(
                    "lume.core.LumeRuntime.hashValue({})",
                    self.emit_expr(receiver, bindings)?
                ))
            }
            core::Expr::Member { receiver, name, .. }
                if matches!(name.as_str(), "isSuccess" | "isSet" | "isDefined")
                    && args.is_empty() =>
            {
                Some(format!(
                    "lume.core.LumeRuntime.extractSuccessIsSet({})",
                    self.emit_expr(receiver, bindings)?
                ))
            }
            core::Expr::Member { receiver, name, .. }
                if matches!(
                    receiver.as_ref(),
                    core::Expr::Member {
                        receiver: owner,
                        name: case,
                        ..
                    } if matches!(owner.as_ref(), core::Expr::Identifier { name: owner_name, .. }
                        if self.bundle.ir.types.iter().any(|ty| {
                            ty.name == *owner_name
                                && ty.enum_cases.iter().any(|item| item.name == *case)
                        }))
                ) =>
            {
                let receiver = self.emit_expr(receiver, bindings)?;
                let args = args
                    .iter()
                    .map(|arg| self.emit_call_arg(arg, bindings))
                    .collect::<Option<Vec<_>>>()?;
                Some(format!(
                    "{receiver}.{}({})",
                    java_member_name(name),
                    args.join(", ")
                ))
            }
            core::Expr::Member { receiver, name, .. } => {
                if let Some(call) = self.source_call(span)
                    && let ir::Callee::Named { path } = &call.callee
                {
                    return self.emit_source_named_call(path, args, call, bindings);
                }
                if self.source_call(span).is_some_and(|call| {
                    matches!(
                        &call.callee,
                        ir::Callee::Method { method, .. } if method == name
                    )
                }) {
                    return self.emit_resolved_member_call(receiver, name, args, span, bindings);
                }
                let receiver_expr = self.emit_receiver_expr(receiver, bindings)?;
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
            core::Expr::Index { .. } => {
                if std::env::var_os("LUME_JAVA_DEBUG_STUBS").is_some() {
                    eprintln!(
                        "readable java cannot classify indexed callee in '{}': {:?}",
                        self.function.name,
                        self.source_call(span).map(|call| &call.callee)
                    );
                }
                None
            }
            _ => None,
        }
    }

    fn emit_source_named_call(
        &self,
        path: &[String],
        args: &[core::CallArg],
        call: &ir::SourceCall,
        bindings: &HashMap<String, String>,
    ) -> Option<String> {
        let ordered_args = self.ordered_source_args(args, call).or_else(|| {
            if std::env::var_os("LUME_JAVA_DEBUG_STUBS").is_some() {
                eprintln!(
                    "readable java cannot order source arguments for '{}': source={:?}, lowered={:?}",
                    path.join("."),
                    args.iter().map(|arg| arg.span).collect::<Vec<_>>(),
                    call.ordered_arg_spans
                );
            }
            None
        })?;
        let emitter = JavaIrSupport::new(self.bundle, self.function, self.names);
        let param_specs = match path {
            [owner]
                if self.names.is_java_type(owner) || emitter.is_lume_constructible_type(owner) =>
            {
                emitter.constructor_param_specs(owner, &call.lowered_args)
            }
            [owner, method] => emitter
                .external_method_param_specs(owner, method, &call.lowered_args)
                .or_else(|| emitter.type_method_param_specs(owner, method, &call.lowered_args)),
            _ => None,
        };
        let emitted = match param_specs.as_deref() {
            Some(specs) => {
                let source_param_count = if call.param_specs.is_empty() {
                    ordered_args.len()
                } else {
                    call.param_specs.len()
                };
                let metadata_evidence_count = specs.len().saturating_sub(source_param_count);
                let source_specs = &specs[..specs.len().saturating_sub(metadata_evidence_count)];
                let mut emitted =
                    self.emit_source_args_for_param_specs(&ordered_args, source_specs, bindings)?;
                let lowered_evidence_count =
                    call.lowered_args.len().saturating_sub(ordered_args.len());
                emitted.extend(self.emit_source_reified_evidence(
                    call,
                    metadata_evidence_count.max(lowered_evidence_count),
                )?);
                emitted
            }
            None => {
                let mut emitted = ordered_args
                    .iter()
                    .map(|arg| self.emit_call_arg(arg, bindings))
                    .collect::<Option<Vec<_>>>()?;
                let evidence_count = call.lowered_args.len().saturating_sub(ordered_args.len());
                emitted.extend(self.emit_source_reified_evidence(call, evidence_count)?);
                emitted
            }
        };
        let joined = emitted.join(", ");

        match path {
            [case] if core_enum_case_owner(case).is_some() => {
                self.emit_core_enum_case(case, &emitted)
            }
            [owner, case]
                if core_enum_case_owner(case).is_some_and(|expected| expected == owner) =>
            {
                self.emit_core_enum_case(case, &emitted)
            }
            [owner, case] if emitter.enum_case(owner, case).is_some() => {
                let generic = self
                    .bundle
                    .ir
                    .types
                    .iter()
                    .find(|ty| ty.name == *owner)
                    .is_some_and(|ty| !ty.type_params.is_empty())
                    .then_some("<>")
                    .unwrap_or("");
                Some(format!(
                    "new {}.{}{generic}({joined})",
                    self.names.named_type(owner),
                    java_type_name(case)
                ))
            }
            [owner] if owner == "Vector" => Some(format!("lume.core.LumeVector.of({joined})")),
            [owner] if owner == "LinkedList" => {
                Some(format!("lume.core.LumeLinkedList.of({joined})"))
            }
            [owner] if owner == "Map" => {
                if emitted.is_empty() {
                    Some("lume.core.LumeMap.empty()".to_string())
                } else {
                    Some(format!("lume.core.LumeMap.fromParts({joined})"))
                }
            }
            [owner] if owner == "Set" && emitted.is_empty() => {
                Some("lume.core.LumeSet.empty()".to_string())
            }
            [owner] if owner == "Range" && emitted.len() == 2 => {
                Some(format!("new lume.core.Range({joined})"))
            }
            [owner] if self.names.is_java_type(owner) => Some(format!(
                "new {}{}({joined})",
                self.names.named_type(owner),
                self.names.java_constructor_type_args(owner)
            )),
            [owner] if emitter.is_lume_constructible_type(owner) => {
                let generic = emitter.lume_constructor_type_args(owner);
                Some(format!(
                    "new {}{generic}({joined})",
                    self.names.named_type(owner)
                ))
            }
            [owner, method] if self.names.is_java_single_type(owner) => Some(format!(
                "{}.INSTANCE.{}({joined})",
                self.names.named_type(owner),
                java_member_name(method)
            )),
            [owner, method] if self.names.is_java_type(owner) => Some(format!(
                "{}.{}({joined})",
                self.names.named_type(owner),
                java_member_name(method)
            )),
            [owner, method] if emitter.is_lume_single_type(owner) => Some(format!(
                "{}.INSTANCE.{}({joined})",
                self.names.named_type(owner),
                java_member_name(method)
            )),
            _ => None,
        }
    }

    fn emit_source_intrinsic_call(
        &self,
        intrinsic: &ir::Intrinsic,
        args: &[core::CallArg],
        call: &ir::SourceCall,
        bindings: &HashMap<String, String>,
    ) -> Option<String> {
        let ordered_args = self.ordered_source_args(args, call)?;
        let emitted = ordered_args
            .iter()
            .enumerate()
            .map(|(index, arg)| {
                let value = self.emit_call_arg(arg, bindings)?;
                if call
                    .param_specs
                    .get(index)
                    .and_then(Option::as_ref)
                    .is_some_and(|spec| spec.lazy)
                {
                    Some(format!("() -> {value}"))
                } else {
                    Some(value)
                }
            })
            .collect::<Option<Vec<_>>>()?;

        match intrinsic {
            ir::Intrinsic::Print => Some(format!(
                "lume.core.LumeRuntime.print({})",
                emitted.join(", ")
            )),
            ir::Intrinsic::Println => Some(format!(
                "lume.core.LumeRuntime.println({})",
                emitted.join(", ")
            )),
            ir::Intrinsic::Printf => Some(format!(
                "lume.core.LumeRuntime.printf({})",
                emitted.join(", ")
            )),
            ir::Intrinsic::Panic => Some(format!(
                "lume.core.LumePanic.panic({})",
                emitted
                    .first()
                    .cloned()
                    .unwrap_or_else(|| java_string_literal("panic"))
            )),
            ir::Intrinsic::Assert => {
                let condition = emitted.first()?;
                let message = emitted
                    .get(1)
                    .cloned()
                    .unwrap_or_else(|| java_string_literal("assertion failed"));
                Some(format!(
                    "lume.core.LumeRuntime.assertTrue({condition}, {message})"
                ))
            }
            ir::Intrinsic::Ensure if emitted.len() == 2 => Some(format!(
                "lume.core.LumeRuntime.ensure({}, {})",
                emitted[0], emitted[1]
            )),
            ir::Intrinsic::Identity if emitted.len() == 1 => emitted.first().cloned(),
            _ => None,
        }
    }

    fn source_call(&self, span: crate::source::Span) -> Option<&ir::SourceCall> {
        self.bundle
            .ir
            .source_calls
            .iter()
            .find(|call| call.function == self.function.id && call.span == span)
            .or_else(|| {
                let mut matches = self
                    .bundle
                    .ir
                    .source_calls
                    .iter()
                    .filter(|call| call.span == span);
                let call = matches.next()?;
                matches.next().is_none().then_some(call)
            })
    }

    fn source_expr_type(&self, span: crate::source::Span) -> Option<ir::Type> {
        self.bundle
            .ir
            .source_exprs
            .iter()
            .find(|source| source.function == self.function.id && source.span == span)
            .or_else(|| {
                let mut matches = self
                    .bundle
                    .ir
                    .source_exprs
                    .iter()
                    .filter(|source| source.span == span);
                let source = matches.next()?;
                matches.next().is_none().then_some(source)
            })
            .filter(|source| !matches!(source.ty, ir::Type::Unknown))
            .map(|source| source.ty.clone())
    }

    fn ordered_source_args<'call>(
        &self,
        args: &'call [core::CallArg],
        call: &ir::SourceCall,
    ) -> Option<Vec<&'call core::CallArg>> {
        call.ordered_arg_spans
            .iter()
            .map(|span| find_source_call_arg(args, *span))
            .collect()
    }

    fn emit_resolved_member_call(
        &self,
        receiver: &core::Expr,
        name: &str,
        args: &[core::CallArg],
        span: crate::source::Span,
        bindings: &HashMap<String, String>,
    ) -> Option<String> {
        let call = self.source_call(span).or_else(|| {
            if std::env::var_os("LUME_JAVA_DEBUG_STUBS").is_some() {
                eprintln!("readable java has no resolved call metadata for member '{name}'");
            }
            None
        })?;
        let ir::Callee::Method {
            receiver: lowered_receiver,
            method,
        } = &call.callee
        else {
            return None;
        };
        if method != name {
            return None;
        }

        let resolved_function = self
            .bundle
            .ir
            .function(call.function)
            .unwrap_or(self.function);
        let ordered_args = self.ordered_source_args(args, call).or_else(|| {
            if std::env::var_os("LUME_JAVA_DEBUG_STUBS").is_some() {
                eprintln!(
                    "readable java cannot order source arguments for member '{name}': source={:?}, lowered={:?}",
                    args.iter().map(|arg| arg.span).collect::<Vec<_>>(),
                    call.ordered_arg_spans
                );
            }
            None
        })?;
        let param_specs = JavaIrSupport::new(self.bundle, resolved_function, self.names)
            .method_param_specs_for_receiver(lowered_receiver, method, &call.lowered_args);
        let Some(param_specs) = param_specs else {
            let mut emitted_args = ordered_args
                .iter()
                .map(|arg| self.emit_call_arg(arg, bindings))
                .collect::<Option<Vec<_>>>()?;
            let evidence_count = call.lowered_args.len().saturating_sub(ordered_args.len());
            emitted_args.extend(self.emit_source_reified_evidence(call, evidence_count)?);
            let receiver_expr = self.emit_receiver_expr(receiver, bindings)?;
            return Some(format!(
                "{}.{}({})",
                receiver_expr,
                java_member_name(name),
                emitted_args.join(", ")
            ));
        };
        let source_param_count = if call.param_specs.is_empty() {
            ordered_args.len()
        } else {
            call.param_specs.len()
        };
        let metadata_evidence_count = param_specs.len().saturating_sub(source_param_count);
        let lowered_evidence_count = call.lowered_args.len().saturating_sub(ordered_args.len());
        let evidence_count = metadata_evidence_count.max(lowered_evidence_count);
        let source_param_specs =
            &param_specs[..param_specs.len().saturating_sub(metadata_evidence_count)];
        if source_param_specs.is_empty() && !ordered_args.is_empty() {
            return None;
        }
        if !param_specs_accept_arg_len(source_param_specs, ordered_args.len()) {
            if std::env::var_os("LUME_JAVA_DEBUG_STUBS").is_some() {
                eprintln!(
                    "readable java resolved incompatible parameter specs for member '{name}': specs={param_specs:?}, args={}",
                    ordered_args.len()
                );
            }
            return None;
        }
        if evidence_count == 0
            && source_param_specs.len() == ordered_args.len()
            && source_param_specs
                .iter()
                .all(|spec| !spec.variadic && spec.coercion.is_none())
        {
            let mut emitted_args = Vec::with_capacity(ordered_args.len());
            for (index, (arg, spec)) in ordered_args.iter().zip(source_param_specs).enumerate() {
                let value = self
                    .emit_source_arg_for_param_spec(arg, bindings, spec)
                    .or_else(|| {
                        if std::env::var_os("LUME_JAVA_DEBUG_STUBS").is_some() {
                            eprintln!(
                                "readable java cannot emit argument {index} for member '{name}': span={:?}, target={:?}",
                                arg.span, spec.ty
                            );
                        }
                        None
                    })?;
                emitted_args.push(value);
            }
            let receiver_expr = self.emit_receiver_expr(receiver, bindings)?;
            return Some(format!(
                "{}.{}({})",
                receiver_expr,
                java_member_name(name),
                emitted_args.join(", ")
            ));
        }
        let mut args = self
            .emit_source_args_for_param_specs(&ordered_args, source_param_specs, bindings)
            .or_else(|| {
                if std::env::var_os("LUME_JAVA_DEBUG_STUBS").is_some() {
                    eprintln!(
                        "readable java cannot emit source arguments for member '{name}': specs={source_param_specs:?}"
                    );
                }
                None
            })?;
        let evidence = self
            .emit_source_reified_evidence(call, evidence_count)
            .or_else(|| {
                if std::env::var_os("LUME_JAVA_DEBUG_STUBS").is_some() {
                    eprintln!(
                        "readable java cannot emit {evidence_count} reified arguments for member '{name}': lowered={:?}",
                        call.lowered_args
                    );
                }
                None
            })?;
        args.extend(evidence);
        let receiver_expr = self.emit_receiver_expr(receiver, bindings).or_else(|| {
            if std::env::var_os("LUME_JAVA_DEBUG_STUBS").is_some() {
                eprintln!("readable java cannot emit receiver for member '{name}': {receiver:?}");
            }
            None
        })?;
        Some(format!(
            "{}.{}({})",
            receiver_expr,
            java_member_name(name),
            args.join(", ")
        ))
    }

    fn emit_source_arg_for_param_spec(
        &self,
        arg: &core::CallArg,
        bindings: &HashMap<String, String>,
        spec: &JavaParamSpec,
    ) -> Option<String> {
        let source = match &arg.value {
            core::Expr::Spread { value, .. } => value.as_ref(),
            value => value,
        };
        if !spec.lazy {
            let value = self
                .emit_expr_against(source, bindings, &spec.ty)
                .or_else(|| {
                    if std::env::var_os("LUME_JAVA_DEBUG_STUBS").is_some() {
                        eprintln!(
                            "readable java cannot emit source argument {source:?} against {:?}",
                            spec.ty
                        );
                    }
                    None
                })?;
            if matches!(source, core::Expr::Lambda { .. })
                && matches!(spec.ty, ir::Type::Function { .. })
            {
                return Some(format!("(({}) ({value}))", self.names.value_type(&spec.ty)));
            }
            let emitter = JavaIrSupport::new(self.bundle, self.function, self.names);
            let value =
                emitter.coerce_to_target_type(value, self.expr_type(source, bindings), &spec.ty);
            return Some(coerce_to_java_primitive(value, spec.coercion));
        }
        if let core::Expr::Identifier { name, .. } = source
            && self.is_lazy_param(name)
        {
            return self.param_reference(name);
        }
        let target_ty = lazy_param_value_type(&spec.ty);
        let value = self
            .emit_expr_against(source, bindings, target_ty)
            .or_else(|| {
                if std::env::var_os("LUME_JAVA_DEBUG_STUBS").is_some() {
                    eprintln!(
                        "readable java cannot emit lazy source argument {source:?} against {target_ty:?}"
                    );
                }
                None
            })?;
        let emitter = JavaIrSupport::new(self.bundle, self.function, self.names);
        let value =
            emitter.coerce_to_target_type(value, self.expr_type(source, bindings), target_ty);
        let value = coerce_to_java_primitive(value, spec.coercion);
        Some(format!("() -> {value}"))
    }

    fn emit_source_args_for_param_specs(
        &self,
        args: &[&core::CallArg],
        specs: &[JavaParamSpec],
        bindings: &HashMap<String, String>,
    ) -> Option<Vec<String>> {
        let Some(variadic_index) = specs.iter().position(|spec| spec.variadic) else {
            if args.len() != specs.len() {
                return None;
            }
            return args
                .iter()
                .zip(specs)
                .map(|(arg, spec)| self.emit_source_arg_for_param_spec(arg, bindings, spec))
                .collect();
        };
        if args.len() < variadic_index {
            return None;
        }

        let mut emitted = args
            .iter()
            .take(variadic_index)
            .zip(specs.iter())
            .map(|(arg, spec)| self.emit_source_arg_for_param_spec(arg, bindings, spec))
            .collect::<Option<Vec<_>>>()?;
        let variadic = specs.get(variadic_index)?;
        let rest = &args[variadic_index..];
        if rest.is_empty() {
            emitted.push(
                variadic
                    .default
                    .as_ref()
                    .map(java_constant)
                    .unwrap_or_else(|| "lume.core.LumeVector.of()".to_string()),
            );
            return Some(emitted);
        }

        if rest.len() == 1 {
            let arg = rest[0];
            let value = match &arg.value {
                core::Expr::Spread { value, .. } => value.as_ref(),
                value => value,
            };
            if matches!(arg.value, core::Expr::Spread { .. })
                || self.expr_type(value, bindings).as_ref() == Some(&variadic.ty)
            {
                emitted.push(self.emit_source_arg_for_param_spec(arg, bindings, variadic)?);
                return Some(emitted);
            }
        }

        let element_ty = variadic_element_type(&variadic.ty)?;
        let has_spread = rest
            .iter()
            .any(|arg| matches!(arg.value, core::Expr::Spread { .. }));
        if !has_spread {
            let items = rest
                .iter()
                .map(|arg| {
                    self.emit_expr_against(&arg.value, bindings, element_ty)
                        .or_else(|| {
                            if std::env::var_os("LUME_JAVA_DEBUG_STUBS").is_some() {
                                eprintln!(
                                    "readable java cannot emit variadic argument {:?} against {element_ty:?}",
                                    arg.value
                                );
                            }
                            None
                        })
                })
                .collect::<Option<Vec<_>>>()?;
            emitted.push(format!("lume.core.LumeVector.of({})", items.join(", ")));
            return Some(emitted);
        }

        let mut packed = "lume.core.LumeVector.empty()".to_string();
        for arg in rest {
            packed = match &arg.value {
                core::Expr::Spread { value, .. } => format!(
                    "{packed}.addAll({})",
                    self.emit_expr_against(value, bindings, &variadic.ty)?
                ),
                value => format!(
                    "{packed}.add({})",
                    self.emit_expr_against(value, bindings, element_ty)?
                ),
            };
        }
        emitted.push(packed);
        Some(emitted)
    }

    fn emit_source_reified_evidence(
        &self,
        call: &ir::SourceCall,
        count: usize,
    ) -> Option<Vec<String>> {
        if count == 0 {
            return Some(Vec::new());
        }
        let resolved_function = self
            .bundle
            .ir
            .function(call.function)
            .unwrap_or(self.function);
        let emitter = JavaIrSupport::new(self.bundle, resolved_function, self.names);
        call.lowered_args
            .iter()
            .rev()
            .take(count)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .map(|operand| {
                if let Some(local_id) = operand_local_id(operand)
                    && let Some(local) = resolved_function.locals.get(local_id.0)
                    && is_reified_type_param_local(&local.name)
                    && call.function == self.function.id
                {
                    return Some(java_local_name(local));
                }
                match emitter.operand_type(operand) {
                    Some(ir::Type::Named { name, args }) if name == "Type" && args.len() == 1 => {
                        Some(type_value_expr(&args[0], self.names))
                    }
                    _ => emitter.emit_operand(operand),
                }
            })
            .collect()
    }

    fn emit_call_arg(
        &self,
        arg: &core::CallArg,
        bindings: &HashMap<String, String>,
    ) -> Option<String> {
        match &arg.value {
            core::Expr::Spread { value, .. } => self.emit_expr(value, bindings),
            _ => self.emit_expr(&arg.value, bindings),
        }
    }

    fn emit_core_enum_case(&self, case: &str, args: &[String]) -> Option<String> {
        let owner = core_enum_case_owner(case)?;
        if case == "None" {
            return args
                .is_empty()
                .then(|| "lume.core.Option.None.instance()".to_string());
        }
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

    fn expr_type(&self, expr: &core::Expr, bindings: &HashMap<String, String>) -> Option<ir::Type> {
        if matches!(expr, core::Expr::Identifier { name, .. } if name == "this") {
            return self.owner.map(|owner| ir::Type::Named {
                name: owner.name.clone(),
                args: owner
                    .type_params
                    .iter()
                    .map(|param| ir::Type::TypeParam(param.clone()))
                    .collect(),
            });
        }
        self.bundle
            .ir
            .source_exprs
            .iter()
            .find(|source| source.function == self.function.id && source.span == expr.span())
            .or_else(|| {
                let mut matches = self
                    .bundle
                    .ir
                    .source_exprs
                    .iter()
                    .filter(|source| source.span == expr.span());
                let source = matches.next()?;
                matches.next().is_none().then_some(source)
            })
            .filter(|source| !matches!(source.ty, ir::Type::Unknown))
            .map(|source| source.ty.clone())
            .or_else(|| self.source_call_return_type(expr.span()))
            .or_else(|| match expr {
                core::Expr::Bool { .. } => Some(ir::Type::Bool),
                core::Expr::Integer { .. } => Some(ir::Type::Int),
                core::Expr::Float { .. } => Some(ir::Type::Float),
                core::Expr::String { .. } => Some(ir::Type::Str),
                core::Expr::Unit { .. } => Some(ir::Type::Unit),
                core::Expr::Unary { expr, .. } => self.expr_type(expr, bindings),
                core::Expr::Member { receiver, name, .. }
                    if matches!(receiver.as_ref(), core::Expr::Identifier { name, .. } if name == "this") =>
                {
                    self.owner.and_then(|owner| {
                        owner
                            .fields
                            .iter()
                            .find(|field| field.name == *name)
                            .map(|field| field.ty.clone())
                    })
                }
                core::Expr::Call { callee, args, .. } => {
                    if let Some(ir::Type::Function { ret, .. }) = self.expr_type(callee, bindings) {
                        return Some(*ret);
                    }
                    let core::Expr::Member { receiver, name, .. } = callee.as_ref() else {
                        return None;
                    };
                    let receiver_ty = self.expr_type(receiver, bindings)?;
                    if let Some(ret) = builtin_method_return_type(&receiver_ty, name, args.len()) {
                        return Some(ret);
                    }
                    let ir::Type::Named {
                        name: receiver_name,
                        args: receiver_args,
                    } = receiver_ty
                    else {
                        return None;
                    };
                    let emitter = JavaIrSupport::new(self.bundle, self.function, self.names);
                    let ret = emitter.type_method_return_type(&receiver_name, name, args.len())?;
                    Some(emitter.substitute_receiver_type_args(
                        &receiver_name,
                        &receiver_args,
                        &ret,
                    ))
                }
                core::Expr::Identifier { name, .. } => self
                    .function
                    .params
                    .iter()
                    .filter_map(|param| self.function.locals.get(param.0))
                    .find(|local| local.name == *name)
                    .map(|local| local.ty.clone())
                    .or_else(|| {
                        self.owner.and_then(|owner| {
                            owner
                                .fields
                                .iter()
                                .find(|field| field.name == *name)
                                .map(|field| field.ty.clone())
                        })
                    }),
                _ => None,
            })
    }

    fn source_call_return_type(&self, span: crate::source::Span) -> Option<ir::Type> {
        let call = self.source_call(span)?;
        let resolved_function = self
            .bundle
            .ir
            .function(call.function)
            .unwrap_or(self.function);
        let emitter = JavaIrSupport::new(self.bundle, resolved_function, self.names);
        match &call.callee {
            ir::Callee::Direct(id) => self
                .bundle
                .ir
                .function(*id)
                .map(|function| function.return_ty.clone()),
            ir::Callee::Method { receiver, method } => {
                emitter.method_return_type_for_receiver(receiver, method, call.lowered_args.len())
            }
            ir::Callee::Named { path } => {
                emitter.named_runtime_call_return_type(path, call.lowered_args.len())
            }
            ir::Callee::Intrinsic(ir::Intrinsic::Identity) => call
                .lowered_args
                .first()
                .and_then(|arg| emitter.operand_type(arg)),
            ir::Callee::Intrinsic(
                ir::Intrinsic::Print
                | ir::Intrinsic::Println
                | ir::Intrinsic::Printf
                | ir::Intrinsic::Assert,
            ) => Some(ir::Type::Unit),
            ir::Callee::Intrinsic(ir::Intrinsic::Ensure) => Some(ir::Type::Named {
                name: "Result".to_string(),
                args: vec![ir::Type::Unit, ir::Type::Unknown],
            }),
            _ => None,
        }
    }

    fn expr_type_with_binding_types(
        &self,
        expr: &core::Expr,
        bindings: &HashMap<String, String>,
        binding_types: &HashMap<String, ir::Type>,
    ) -> Option<ir::Type> {
        if let Some(ty) = self.expr_type(expr, bindings) {
            return Some(ty);
        }
        match expr {
            core::Expr::Identifier { name, .. } => binding_types.get(name).cloned(),
            core::Expr::Call { callee, args, .. } => {
                let member = match callee.as_ref() {
                    core::Expr::Member { .. } => callee.as_ref(),
                    core::Expr::Index { receiver, .. }
                        if matches!(receiver.as_ref(), core::Expr::Member { .. }) =>
                    {
                        receiver.as_ref()
                    }
                    _ => return None,
                };
                let core::Expr::Member { receiver, name, .. } = member else {
                    unreachable!();
                };
                let receiver_ty =
                    self.expr_type_with_binding_types(receiver, bindings, binding_types)?;
                if let Some(ret) = builtin_method_return_type(&receiver_ty, name, args.len()) {
                    return Some(ret);
                }
                let ir::Type::Named {
                    name: receiver_name,
                    args: receiver_args,
                } = receiver_ty
                else {
                    return None;
                };
                let emitter = JavaIrSupport::new(self.bundle, self.function, self.names);
                let ret = emitter
                    .type_method_return_type(&receiver_name, name, args.len())
                    .or_else(|| {
                        self.names
                            .java_method_return_type(&receiver_name, name, args.len())
                    })?;
                Some(emitter.substitute_receiver_type_args(&receiver_name, &receiver_args, &ret))
            }
            _ => None,
        }
    }

    fn emit_receiver_expr(
        &self,
        receiver: &core::Expr,
        bindings: &HashMap<String, String>,
    ) -> Option<String> {
        let mut receiver_expr = self.emit_expr(receiver, bindings)?;
        let Some(mut receiver_ty) = self.expr_type(receiver, bindings) else {
            return Some(receiver_expr);
        };

        if let Some(param) = self.type_param_name(&receiver_ty)
            && let Some(bound) = self.generic_bound_for_type_param(param)
        {
            receiver_ty = bound;
        }

        let declared_ty = match receiver {
            core::Expr::Identifier { name, .. } => {
                let reference = bindings
                    .get(name)
                    .cloned()
                    .or_else(|| self.param_reference(name));
                reference.and_then(|reference| {
                    self.function
                        .locals
                        .iter()
                        .find(|local| java_local_name(local) == reference)
                        .map(|local| local.ty.clone())
                })
            }
            _ => None,
        };
        let receiver_java_type = self.names.value_type(&receiver_ty);
        let needs_cast = declared_ty
            .as_ref()
            .map(|declared| self.names.value_type(declared) != receiver_java_type)
            .unwrap_or_else(|| {
                self.type_param_name(
                    &self
                        .expr_type(receiver, bindings)
                        .unwrap_or(ir::Type::Unknown),
                )
                .is_some()
            });
        if needs_cast && receiver_java_type != "Object" {
            receiver_expr = format!("(({receiver_java_type}) {receiver_expr})");
        }
        Some(receiver_expr)
    }

    fn emit_inline_block(
        &self,
        block: &core::Block,
        mut bindings: HashMap<String, String>,
        mut binding_types: HashMap<String, ir::Type>,
        returns_tail: bool,
    ) -> Option<String> {
        let mut out = String::from("{\n");
        self.emit_statement_block(
            &mut out,
            block,
            "            ",
            &mut bindings,
            &mut binding_types,
            &mut HashSet::new(),
            returns_tail,
            0,
        )?;
        out.push_str("        }");
        Some(out)
    }

    fn pattern_binding_expr_type(
        &self,
        expr: &core::Expr,
        binding_types: &HashMap<String, ir::Type>,
    ) -> Option<ir::Type> {
        match expr {
            core::Expr::Identifier { name, .. } => binding_types.get(name).cloned(),
            _ => None,
        }
    }

    fn type_param_name<'ty>(&self, ty: &'ty ir::Type) -> Option<&'ty str> {
        match ty {
            ir::Type::TypeParam(name) => Some(name),
            ir::Type::Named { name, args }
                if args.is_empty()
                    && (self.function.type_params.contains(name)
                        || self
                            .owner
                            .is_some_and(|owner| owner.type_params.contains(name))) =>
            {
                Some(name)
            }
            _ => None,
        }
    }

    fn generic_conditions(&self) -> impl Iterator<Item = &ir::GenericCondition> {
        self.function.generic_conditions.iter().chain(
            self.owner
                .into_iter()
                .flat_map(|owner| owner.generic_conditions.iter()),
        )
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
    structured_source_function_body(bundle, function, names)
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
    let body = structured_source_function_body(bundle, function, names).unwrap_or_else(|| {
        let mut body = String::new();
        push_stub_body(&mut body);
        body
    });
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
    match structured_source_constructor_body(
        bundle,
        function,
        names,
        ty.field_init.map(|_| "this.__lume_field_init();"),
    ) {
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

struct JavaIrSupport<'a> {
    bundle: &'a BackendBundle,
    function: &'a ir::Function,
    names: &'a JavaNames,
    inferred_local_types: HashMap<ir::LocalId, ir::Type>,
}

impl<'a> JavaIrSupport<'a> {
    fn new(bundle: &'a BackendBundle, function: &'a ir::Function, names: &'a JavaNames) -> Self {
        let mut emitter = Self {
            bundle,
            function,
            names,
            inferred_local_types: HashMap::new(),
        };
        emitter.infer_local_types();
        emitter
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

    fn emit_enum_case_call(
        &self,
        enum_name: &str,
        case_name: &str,
        operands: &[ir::Operand],
    ) -> Option<String> {
        let args = self.emit_operands(operands)?;
        if self.enum_case(enum_name, case_name)?.kind == TypeKind::Object {
            return args.is_empty().then(|| {
                format!(
                    "{}.{}.instance()",
                    self.names.named_type(enum_name),
                    java_type_name(case_name)
                )
            });
        }
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
            let body = structured_source_function_body_with_local_overrides(
                self.bundle,
                function,
                self.names,
                &capture_overrides,
            )?;
            out.push_str(&body);
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

        for method in methods {
            let function = self.bundle.ir.function(method.function)?;
            let capture_overrides =
                self.push_anonymous_interface_capture_fields(&mut out, method, function, None)?;
            out.push_str("        @Override\n        public ");
            push_function_signature_named(&mut out, function, self.names, &method.name);
            let body = structured_source_function_body_with_local_overrides(
                self.bundle,
                function,
                self.names,
                &capture_overrides,
            )?;
            out.push_str(&body);
        }

        out.push_str("    }");
        Some(out)
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

    fn emit_operand(&self, operand: &ir::Operand) -> Option<String> {
        match operand {
            ir::Operand::Copy(place) | ir::Operand::Move(place) => self.emit_place(place),
            ir::Operand::Const(constant) => Some(java_constant(constant)),
        }
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
                // These runtime helpers return Object/LumeVector<?> at the Java ABI.
                // Keep the source unknown so assignment emits the typed IR cast.
                ir::Callee::Intrinsic(ir::Intrinsic::ListGet | ir::Intrinsic::ListSlice) => {
                    Some(ir::Type::Unknown)
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
                | ir::Callee::Intrinsic(ir::Intrinsic::VariantField(_))
                | ir::Callee::Intrinsic(ir::Intrinsic::PatternField(_)) => Some(ir::Type::Unknown),
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
            | ir::BinaryOp::IdentityEq
            | ir::BinaryOp::IdentityNotEq
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
        if source_ty
            .as_ref()
            .is_some_and(|source| self.names.value_type(source) == self.names.value_type(target_ty))
        {
            return expr;
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
        if enum_case_view_parts(name).is_some() {
            return false;
        }
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
        if matches!(local.kind, ir::LocalKind::Capture) && local.name == "this" {
            "this".to_string()
        } else {
            java_local_name(local)
        }
    }
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
            ir::Type::Union(_) => "Object".to_string(),
            ir::Type::Function { params, ret } => self.function_type(params, ret),
            ir::Type::Named { name, args } if enum_case_view_parts(name).is_some() => {
                self.enum_case_view_type(name, args)
            }
            ir::Type::Named { name, .. } if is_reflection_type(name) => {
                "lume.core.LumeType".to_string()
            }
            ir::Type::Named { name, args } if args.is_empty() => java_named_builtin_value(name)
                .or_else(|| self.java_types.get(name).cloned())
                .unwrap_or_else(|| java_type_name(name)),
            ir::Type::Named { name, args } if name == "Eq" => format!(
                "lume.core.Eq<{}>",
                args.iter()
                    .map(|arg| self.value_type(arg))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            ir::Type::Named { name, args } if name == "Hashed" => format!(
                "lume.core.Hashed<{}>",
                args.iter()
                    .map(|arg| self.value_type(arg))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
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

    fn enum_case_view_type(&self, name: &str, args: &[ir::Type]) -> String {
        let (owner, case_name) = enum_case_view_parts(name)
            .expect("enum case view type checked before Java type rendering");
        let owner = match owner {
            "Option" | "Result" | "Either" => format!("lume.core.{owner}"),
            _ => self.named_type(owner),
        };
        let args = args
            .iter()
            .map(|arg| self.value_type(arg))
            .collect::<Vec<_>>()
            .join(", ");
        if args.is_empty() {
            format!("{owner}.{}", java_type_name(case_name))
        } else {
            format!("{owner}.{}<{args}>", java_type_name(case_name))
        }
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
        "Eq" => Some("lume.core.Eq".to_string()),
        "Hashed" => Some("lume.core.Hashed".to_string()),
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

fn core_enum_case_payload_type(case: &str, ty: &ir::Type) -> Option<ir::Type> {
    let ir::Type::Named { name, args } = ty else {
        return None;
    };
    match (case, name.as_str(), args.as_slice()) {
        ("Some", "Option", [value]) => Some(value.clone()),
        ("Ok", "Result", [value, _]) => Some(value.clone()),
        ("Err", "Result", [_, error]) => Some(error.clone()),
        ("Left", "Either", [left, _]) => Some(left.clone()),
        ("Right", "Either", [_, right]) => Some(right.clone()),
        _ => None,
    }
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
                ("slice", 0) if name == "Vector" => Some(Vec::new()),
                ("slice", 1) if name == "Vector" => Some(vec![ir::Type::Int]),
                ("slice", 2) if name == "Vector" => Some(vec![ir::Type::Int, ir::Type::Int]),
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
        ir::Type::Named { name, .. }
            if matches!(
                name.as_str(),
                "Type"
                    | "ClassType"
                    | "ShapeType"
                    | "EnumType"
                    | "InterfaceType"
                    | "ObjectType"
                    | "AnnotationType"
                    | "Field"
                    | "Method"
                    | "EnumCase"
            ) && method == "annotation"
                && arg_len == 0 =>
        {
            Some(ir::Type::Named {
                name: "Option".to_string(),
                args: vec![ir::Type::named("AnnotationValue")],
            })
        }
        ir::Type::Named { name, .. }
            if matches!(
                name.as_str(),
                "Type"
                    | "ClassType"
                    | "ShapeType"
                    | "EnumType"
                    | "InterfaceType"
                    | "ObjectType"
                    | "AnnotationType"
                    | "Field"
                    | "Method"
                    | "EnumCase"
            ) && method == "hasAnnotation"
                && arg_len == 0 =>
        {
            Some(ir::Type::Bool)
        }
        ir::Type::Named { name, args }
            if name == "Vector" && args.len() == 1 && method == "slice" && arg_len <= 2 =>
        {
            Some(receiver.clone())
        }
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
        ir::Type::Named { name, args } if name == "Map" && args.len() == 2 => {
            match (method, arg_len) {
                ("get", 1) => Some(ir::Type::Named {
                    name: "Option".to_string(),
                    args: vec![args[1].clone()],
                }),
                ("put", 2) => Some(receiver.clone()),
                _ => None,
            }
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

fn enum_case_view_parts(name: &str) -> Option<(&str, &str)> {
    let (owner, case_name) = name.split_once("::")?;
    (!owner.is_empty() && !case_name.is_empty()).then_some((owner, case_name))
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

fn source_literal_type(expr: &core::Expr) -> Option<ir::Type> {
    match expr {
        core::Expr::Integer { .. } => Some(ir::Type::Int),
        core::Expr::Float { .. } => Some(ir::Type::Float),
        core::Expr::String { .. } => Some(ir::Type::Str),
        core::Expr::Bool { .. } => Some(ir::Type::Bool),
        core::Expr::Unit { .. } => Some(ir::Type::Unit),
        core::Expr::Spread { value, .. } => source_literal_type(value),
        _ => None,
    }
}

fn emit_pattern_literal(expr: &ast::Expr) -> Option<String> {
    match expr {
        ast::Expr::Integer { raw, .. } => Some(format!("{raw}L")),
        ast::Expr::Float { raw, .. } => Some(raw.clone()),
        ast::Expr::String { raw, .. } => {
            Some(java_string_literal(&decode_lume_string_literal(raw)))
        }
        ast::Expr::Bool { value, .. } => Some(value.to_string()),
        ast::Expr::Unit { .. } => Some("lume.core.LumeUnit.INSTANCE".to_string()),
        ast::Expr::Unary {
            op: ast::UnaryOp::Neg,
            expr,
            ..
        } => Some(format!("(-{})", emit_pattern_literal(expr)?)),
        ast::Expr::Group { inner, .. } => emit_pattern_literal(inner),
        _ => None,
    }
}

fn core_enum_case_fields(
    case_name: &str,
    value_ty: &ir::Type,
) -> Option<Vec<(&'static str, ir::Type)>> {
    let owner = core_enum_case_owner(case_name)?;
    let ir::Type::Named { name, args } = value_ty else {
        return None;
    };
    if name != owner {
        return None;
    }
    match (owner, case_name, args.as_slice()) {
        ("Option", "Some", [value]) => Some(vec![("value", value.clone())]),
        ("Option", "None", [_]) => Some(Vec::new()),
        ("Result", "Ok", [value, _]) => Some(vec![("value", value.clone())]),
        ("Result", "Err", [_, error]) => Some(vec![("error", error.clone())]),
        ("Either", "Left", [left, _]) => Some(vec![("value", left.clone())]),
        ("Either", "Right", [_, right]) => Some(vec![("value", right.clone())]),
        _ => None,
    }
}

fn lifted_success_type(ty: &ir::Type) -> Option<ir::Type> {
    match ty {
        ir::Type::Named { name, args } if name == "Option" && args.len() == 1 => {
            args.first().cloned()
        }
        ir::Type::Named { name, args } if name == "Result" && args.len() == 2 => {
            args.first().cloned()
        }
        ir::Type::Named { name, args } if name == "Either" && args.len() == 2 => {
            args.get(1).cloned()
        }
        _ => None,
    }
}

fn find_source_call_arg(
    args: &[core::CallArg],
    span: crate::source::Span,
) -> Option<&core::CallArg> {
    args.iter().find_map(|arg| {
        if arg.span == span {
            return Some(arg);
        }
        match &arg.value {
            core::Expr::RecordLiteral { fields, .. } => find_source_call_arg(fields, span),
            _ => None,
        }
    })
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
        ir::Type::Named { args, .. } | ir::Type::Tuple(args) | ir::Type::Union(args) => {
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
        ir::Type::Named { args, .. } | ir::Type::Tuple(args) | ir::Type::Union(args) => {
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
        | ir::Type::Union(_)
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
        ir::Type::Named { args, .. } | ir::Type::Tuple(args) | ir::Type::Union(args) => args
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
        TypeRef::Union { members, .. } => {
            ir::Type::Union(members.iter().map(type_ref_to_ir).collect())
        }
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
