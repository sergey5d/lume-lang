use std::path::{Path, PathBuf};

use crate::{
    ast::TypeKind,
    backend::BackendBundle,
    ir::{self, FunctionKind},
};

pub(crate) struct JavaSource {
    pub(crate) relative_path: PathBuf,
    pub(crate) contents: String,
}

pub(crate) fn render_declaration_skeletons(bundle: &BackendBundle) -> Vec<JavaSource> {
    let package = JavaPackage::from_module(bundle.ir.module.as_deref());
    let mut sources = Vec::new();

    sources.push(JavaSource {
        relative_path: package.relative_file(&format!("{}.java", module_class_name(bundle))),
        contents: render_module_wrapper(bundle, &package),
    });

    for ty in &bundle.ir.types {
        sources.push(JavaSource {
            relative_path: package.relative_file(&format!("{}.java", java_type_name(&ty.name))),
            contents: render_type_shell(bundle, ty, &package),
        });
    }

    sources
}

fn render_module_wrapper(bundle: &BackendBundle, package: &JavaPackage) -> String {
    let mut out = String::new();
    push_header(&mut out, package);
    out.push_str(&format!("final class {} {{\n", module_class_name(bundle)));
    out.push_str("    private ");
    out.push_str(&module_class_name(bundle));
    out.push_str("() {}\n");

    for global in &bundle.ir.globals {
        out.push('\n');
        out.push_str("    static ");
        out.push_str(&java_type_for_value(&global.ty));
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
        push_function_signature(&mut out, function);
        push_stub_body(&mut out);
    }

    out.push_str("}\n");
    out
}

fn render_type_shell(bundle: &BackendBundle, ty: &ir::TypeDef, package: &JavaPackage) -> String {
    match ty.kind {
        TypeKind::Annotation => render_annotation(ty, package),
        TypeKind::Class => render_class(bundle, ty, package),
        TypeKind::Record => render_shape(bundle, ty, package),
        TypeKind::Single => render_single(bundle, ty, package),
        TypeKind::Interface => render_interface(bundle, ty, package),
        TypeKind::Enum => render_enum(bundle, ty, package),
    }
}

fn render_class(bundle: &BackendBundle, ty: &ir::TypeDef, package: &JavaPackage) -> String {
    let mut out = String::new();
    push_header(&mut out, package);
    out.push_str(&format!(
        "class {}{} {{\n",
        java_type_name(&ty.name),
        java_type_params(&ty.type_params)
    ));
    push_fields(&mut out, ty);
    push_instance_methods(&mut out, bundle, ty, MethodShell::StubBody);
    out.push_str("}\n");
    out
}

fn render_shape(bundle: &BackendBundle, ty: &ir::TypeDef, package: &JavaPackage) -> String {
    let mut out = String::new();
    push_header(&mut out, package);
    out.push_str(&format!(
        "record {}{}({}) {{\n",
        java_type_name(&ty.name),
        java_type_params(&ty.type_params),
        ty.fields
            .iter()
            .map(|field| format!(
                "{} {}",
                java_type_for_value(&field.ty),
                java_member_name(&field.name)
            ))
            .collect::<Vec<_>>()
            .join(", ")
    ));
    push_instance_methods(&mut out, bundle, ty, MethodShell::StubBody);
    out.push_str("}\n");
    out
}

fn render_single(bundle: &BackendBundle, ty: &ir::TypeDef, package: &JavaPackage) -> String {
    let mut out = String::new();
    let name = java_type_name(&ty.name);
    push_header(&mut out, package);
    out.push_str(&format!("final class {name} {{\n"));
    out.push_str(&format!(
        "    static final {name} INSTANCE = new {name}();\n"
    ));
    out.push_str(&format!("    private {name}() {{}}\n"));
    push_fields(&mut out, ty);
    push_instance_methods(&mut out, bundle, ty, MethodShell::StubBody);
    out.push_str("}\n");
    out
}

fn render_interface(bundle: &BackendBundle, ty: &ir::TypeDef, package: &JavaPackage) -> String {
    let mut out = String::new();
    push_header(&mut out, package);
    out.push_str(&format!(
        "interface {}{} {{\n",
        java_type_name(&ty.name),
        java_type_params(&ty.type_params)
    ));
    push_instance_methods(&mut out, bundle, ty, MethodShell::Abstract);
    out.push_str("}\n");
    out
}

fn render_annotation(ty: &ir::TypeDef, package: &JavaPackage) -> String {
    let mut out = String::new();
    push_header(&mut out, package);
    out.push_str(&format!("@interface {} {{\n", java_type_name(&ty.name)));
    for field in &ty.fields {
        out.push_str("    ");
        out.push_str(&java_type_for_annotation(&field.ty));
        out.push(' ');
        out.push_str(&java_member_name(&field.name));
        out.push_str("();\n");
    }
    out.push_str("}\n");
    out
}

fn render_enum(bundle: &BackendBundle, ty: &ir::TypeDef, package: &JavaPackage) -> String {
    let mut out = String::new();
    let enum_name = java_type_name(&ty.name);
    let type_params = java_type_params(&ty.type_params);
    let type_args = java_type_args(&ty.type_params);
    push_header(&mut out, package);

    if ty.enum_cases.is_empty() {
        out.push_str(&format!("interface {enum_name}{type_params} {{\n"));
    } else {
        let permits = ty
            .enum_cases
            .iter()
            .map(|case| format!("{enum_name}.{}", java_type_name(&case.name)))
            .collect::<Vec<_>>()
            .join(", ");
        out.push_str(&format!(
            "sealed interface {enum_name}{type_params} permits {permits} {{\n"
        ));
    }

    push_instance_methods(&mut out, bundle, ty, MethodShell::DefaultBody);

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
                        java_type_for_value(&field.ty),
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

fn push_header(out: &mut String, package: &JavaPackage) {
    out.push_str("// Generated by Lume Java backend.\n");
    if let Some(name) = &package.name {
        out.push_str("package ");
        out.push_str(name);
        out.push_str(";\n\n");
    }
}

fn push_fields(out: &mut String, ty: &ir::TypeDef) {
    for field in &ty.fields {
        out.push_str("    ");
        out.push_str(&java_type_for_value(&field.ty));
        out.push(' ');
        out.push_str(&java_member_name(&field.name));
        out.push_str(";\n");
    }
}

fn push_instance_methods(
    out: &mut String,
    bundle: &BackendBundle,
    ty: &ir::TypeDef,
    shell: MethodShell,
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
        if shell == MethodShell::DefaultBody {
            out.push_str("default ");
        }
        push_function_signature(out, function);
        match shell {
            MethodShell::Abstract => out.push_str(";\n"),
            MethodShell::DefaultBody | MethodShell::StubBody => push_stub_body(out),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MethodShell {
    Abstract,
    DefaultBody,
    StubBody,
}

fn push_function_signature(out: &mut String, function: &ir::Function) {
    if !function.type_params.is_empty() {
        out.push_str(&java_type_params(&function.type_params));
        out.push(' ');
    }
    out.push_str(&java_type_for_return(&function.return_ty));
    out.push(' ');
    out.push_str(&java_member_name(&function.name));
    out.push('(');
    out.push_str(
        &function
            .params
            .iter()
            .filter_map(|param| function.locals.get(param.0))
            .map(|local| {
                format!(
                    "{} {}",
                    java_type_for_value(&local.ty),
                    java_member_name(&local.name)
                )
            })
            .collect::<Vec<_>>()
            .join(", "),
    );
    out.push(')');
}

fn push_stub_body(out: &mut String) {
    out.push_str(" {\n");
    out.push_str("        throw new UnsupportedOperationException(\"Lume Java body generation is not implemented yet\");\n");
    out.push_str("    }\n");
}

fn module_class_name(bundle: &BackendBundle) -> String {
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
    format!("{}Module", java_type_name(&raw))
}

fn java_type_for_return(ty: &ir::Type) -> String {
    match ty {
        ir::Type::Unit => "void".to_string(),
        ir::Type::Named { name, args } if name == "Unit" && args.is_empty() => "void".to_string(),
        _ => java_type_for_value(ty),
    }
}

fn java_type_for_value(ty: &ir::Type) -> String {
    match ty {
        ir::Type::Unknown => "Object".to_string(),
        ir::Type::Never => "lume.runtime.LumePanic".to_string(),
        ir::Type::Unit => "lume.runtime.LumeUnit".to_string(),
        ir::Type::Bool => "Boolean".to_string(),
        ir::Type::Int => "Long".to_string(),
        ir::Type::Float => "Double".to_string(),
        ir::Type::Str => "String".to_string(),
        ir::Type::Named { name, args } if args.is_empty() => {
            java_named_builtin_value(name).unwrap_or_else(|| java_type_name(name))
        }
        ir::Type::Named { name, args } if is_builtin_container(name) => {
            java_builtin_container(name, args)
        }
        ir::Type::Named { name, args } => {
            let args = args
                .iter()
                .map(java_type_for_value)
                .collect::<Vec<_>>()
                .join(", ");
            format!("{}<{args}>", java_type_name(name))
        }
        ir::Type::Tuple(items) => java_tuple_type(items),
        ir::Type::Record(_) | ir::Type::Function { .. } => "Object".to_string(),
        ir::Type::TypeParam(name) => java_type_name(name),
    }
}

fn java_type_for_annotation(ty: &ir::Type) -> String {
    match ty {
        ir::Type::Bool => "boolean".to_string(),
        ir::Type::Int => "long".to_string(),
        ir::Type::Float => "double".to_string(),
        ir::Type::Str => "String".to_string(),
        ir::Type::Named { name, args } if args.is_empty() => {
            java_named_builtin_annotation(name).unwrap_or_else(|| java_type_name(name))
        }
        ir::Type::Named { name, args } if name == "List" && args.len() == 1 => {
            format!("{}[]", java_type_for_annotation(&args[0]))
        }
        _ => "String".to_string(),
    }
}

fn java_named_builtin_value(name: &str) -> Option<String> {
    match name {
        "Unit" => Some("lume.runtime.LumeUnit".to_string()),
        "Bool" => Some("Boolean".to_string()),
        "Int" => Some("Long".to_string()),
        "Float" => Some("Double".to_string()),
        "Str" => Some("String".to_string()),
        "Rune" => Some("Integer".to_string()),
        _ => None,
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
        "Array" | "Either" | "List" | "Map" | "Option" | "Result" | "Set"
    )
}

fn java_builtin_container(name: &str, args: &[ir::Type]) -> String {
    match name {
        "Array" if args.len() == 1 => {
            format!("lume.runtime.LumeArray<{}>", java_type_for_value(&args[0]))
        }
        "Either" if args.len() == 2 => format!(
            "lume.runtime.Either<{}, {}>",
            java_type_for_value(&args[0]),
            java_type_for_value(&args[1])
        ),
        "List" if args.len() == 1 => {
            format!("lume.runtime.LumeList<{}>", java_type_for_value(&args[0]))
        }
        "Map" if args.len() == 2 => format!(
            "lume.runtime.LumeMap<{}, {}>",
            java_type_for_value(&args[0]),
            java_type_for_value(&args[1])
        ),
        "Option" if args.len() == 1 => {
            format!("lume.runtime.Option<{}>", java_type_for_value(&args[0]))
        }
        "Result" if args.len() == 2 => format!(
            "lume.runtime.Result<{}, {}>",
            java_type_for_value(&args[0]),
            java_type_for_value(&args[1])
        ),
        "Set" if args.len() == 1 => {
            format!("lume.runtime.LumeSet<{}>", java_type_for_value(&args[0]))
        }
        _ => "Object".to_string(),
    }
}

fn java_tuple_type(items: &[ir::Type]) -> String {
    if !(2..=8).contains(&items.len()) {
        return "Object".to_string();
    }
    let args = items
        .iter()
        .map(java_type_for_value)
        .collect::<Vec<_>>()
        .join(", ");
    format!("lume.runtime.Tuple{}<{args}>", items.len())
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

fn java_member_name(name: &str) -> String {
    sanitize_identifier(name, IdentifierStyle::Member)
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
