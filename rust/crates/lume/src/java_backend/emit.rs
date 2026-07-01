use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use crate::{
    ast::TypeKind,
    backend::{BackendBundle, DescriptorOrigin},
    ir::{self, FunctionKind},
};

pub(crate) struct JavaSource {
    pub(crate) relative_path: PathBuf,
    pub(crate) contents: String,
}

pub(crate) fn render_declaration_skeletons(bundle: &BackendBundle) -> Vec<JavaSource> {
    let package = JavaPackage::from_module(bundle.ir.module.as_deref());
    let names = JavaNames::from_bundle(bundle);
    let mut sources = Vec::new();

    sources.push(JavaSource {
        relative_path: package.relative_file(&format!("{}.java", module_class_name(bundle))),
        contents: render_module_wrapper(bundle, &package, &names),
    });

    if let Some(entrypoint) = render_entrypoint_runner(bundle, &package) {
        sources.push(entrypoint);
    }

    for ty in &bundle.ir.types {
        if names.is_java_type(&ty.name) {
            continue;
        }
        sources.push(JavaSource {
            relative_path: package.relative_file(&format!("{}.java", java_type_name(&ty.name))),
            contents: render_type_shell(bundle, ty, &package, &names),
        });
    }

    sources
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
    match ty.kind {
        TypeKind::Annotation => render_annotation(ty, package, names),
        TypeKind::Class => render_class(bundle, ty, package, names),
        TypeKind::Record => render_shape(bundle, ty, package, names),
        TypeKind::Single => render_single(bundle, ty, package, names),
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
        "class {}{} {{\n",
        java_type_name(&ty.name),
        java_type_params(&ty.type_params)
    ));
    push_fields(&mut out, ty, names);
    push_class_constructor(&mut out, ty, names);
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
        "record {}{}({}) {{\n",
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
            .join(", ")
    ));
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
    out.push_str(&format!("final class {name} {{\n"));
    out.push_str(&format!(
        "    static final {name} INSTANCE = new {name}();\n"
    ));
    out.push_str(&format!("    private {name}() {{}}\n"));
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
        "interface {}{} {{\n",
        java_type_name(&ty.name),
        java_type_params(&ty.type_params)
    ));
    push_instance_methods(&mut out, bundle, ty, MethodShell::Abstract, names);
    out.push_str("}\n");
    out
}

fn render_annotation(ty: &ir::TypeDef, package: &JavaPackage, names: &JavaNames) -> String {
    let mut out = String::new();
    push_header(&mut out, package);
    out.push_str(&format!("@interface {} {{\n", java_type_name(&ty.name)));
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
        if shell == MethodShell::DefaultBody {
            out.push_str("default ");
        }
        push_function_signature(out, function, names);
        match shell {
            MethodShell::Abstract => out.push_str(";\n"),
            MethodShell::DefaultBody | MethodShell::StubBody => {
                push_function_body(out, bundle, function, names)
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
    if !function.type_params.is_empty() {
        out.push_str(&java_type_params(&function.type_params));
        out.push(' ');
    }
    out.push_str(&names.return_type(&function.return_ty));
    out.push(' ');
    out.push_str(&java_member_name(&function.name));
    out.push('(');
    out.push_str(
        &function
            .params
            .iter()
            .filter_map(|param| function.locals.get(param.0))
            .map(|local| format!("{} {}", names.value_type(&local.ty), java_local_name(local)))
            .collect::<Vec<_>>()
            .join(", "),
    );
    out.push(')');
}

fn push_function_body(
    out: &mut String,
    bundle: &BackendBundle,
    function: &ir::Function,
    names: &JavaNames,
) {
    match FunctionEmitter::new(bundle, function, names).emit_body() {
        Some(body) => out.push_str(&body),
        None => push_stub_body(out),
    }
}

fn push_stub_body(out: &mut String) {
    out.push_str(" {\n");
    out.push_str("        throw new UnsupportedOperationException(\"Lume Java body generation is not implemented yet\");\n");
    out.push_str("    }\n");
}

fn push_class_constructor(out: &mut String, ty: &ir::TypeDef, names: &JavaNames) {
    let name = java_type_name(&ty.name);
    out.push('\n');
    out.push_str("    ");
    out.push_str(&name);
    out.push_str("() {}\n");
    if ty.fields.is_empty() {
        return;
    }

    out.push('\n');
    out.push_str("    ");
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
}

impl<'a> FunctionEmitter<'a> {
    fn new(bundle: &'a BackendBundle, function: &'a ir::Function, names: &'a JavaNames) -> Self {
        Self {
            bundle,
            function,
            names,
            module_class: module_class_name(bundle),
        }
    }

    fn emit_body(&self) -> Option<String> {
        let mut out = String::new();
        out.push_str(" {\n");
        for local in &self.function.locals {
            if self.local_is_declared_elsewhere(local) {
                continue;
            }
            out.push_str("        ");
            out.push_str(&self.names.value_type(&local.ty));
            out.push(' ');
            out.push_str(&java_local_name(local));
            out.push_str(" = ");
            out.push_str(&java_default_value(&local.ty));
            out.push_str(";\n");
        }

        if !self.function.blocks.is_empty() {
            out.push_str("        int __block = ");
            out.push_str(&self.function.entry.0.to_string());
            out.push_str(";\n");
            out.push_str("        while (true) {\n");
            out.push_str("            switch (__block) {\n");
            for block in &self.function.blocks {
                self.emit_block(&mut out, block)?;
            }
            out.push_str("                default:\n");
            out.push_str("                    throw new IllegalStateException(\"unknown Lume block \" + __block);\n");
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
                let mut value_expr = self.emit_rvalue(value)?;
                if let Some(target_ty) = self.place_type(target) {
                    value_expr =
                        self.coerce_to_target_type(value_expr, self.rvalue_type(value), &target_ty);
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
            ir::StatementKind::Defer { .. } => None,
        }
    }

    fn emit_terminator(&self, out: &mut String, terminator: &ir::Terminator) -> Option<()> {
        match &terminator.kind {
            ir::TerminatorKind::Goto(target) => {
                out.push_str("                    __block = ");
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
                out.push_str("                        __block = ");
                out.push_str(&then_block.0.to_string());
                out.push_str(";\n");
                out.push_str("                    } else {\n");
                out.push_str("                        __block = ");
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
                    out.push_str("                        __block = ");
                    out.push_str(&arm.target.0.to_string());
                    out.push_str(";\n");
                    out.push_str("                        break;\n");
                    out.push_str("                    }\n");
                }
                out.push_str("                    __block = ");
                out.push_str(&default.0.to_string());
                out.push_str(";\n");
                Some(())
            }
            ir::TerminatorKind::Return(value) => {
                if is_java_void_type(&self.function.return_ty) {
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
            ir::SwitchValue::EnumCase(_) => None,
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
                Some(format!("lume.runtime.LumeList.of({})", args.join(", ")))
            }
            ir::RValue::Construct { ty, fields } => self.emit_construct(ty, fields),
            ir::RValue::Variant {
                enum_name,
                case_name,
                fields,
            } => self.emit_variant(enum_name, case_name, fields),
            ir::RValue::Field { base, name } => Some(format!(
                "{}.{}",
                self.emit_operand(base)?,
                java_member_name(name)
            )),
            ir::RValue::Cast { operand, .. } => self.emit_operand(operand),
            ir::RValue::NamedValue { .. }
            | ir::RValue::Record(_)
            | ir::RValue::RecordUpdate { .. }
            | ir::RValue::Index { .. }
            | ir::RValue::TypeTest { .. }
            | ir::RValue::TypeOf { .. }
            | ir::RValue::Closure { .. } => None,
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
            ir::BinaryOp::RecordMerge => None,
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
                    ir::BinaryOp::Eq | ir::BinaryOp::NotEq | ir::BinaryOp::RecordMerge => {
                        unreachable!()
                    }
                };
                Some(format!("({left} {op} {right})"))
            }
        }
    }

    fn emit_call(&self, callee: &ir::Callee, args: &[ir::Operand]) -> Option<String> {
        let args = self.emit_operands(args)?;
        match callee {
            ir::Callee::Direct(id) => {
                let target = self.bundle.ir.function(*id)?;
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
                    _ => None,
                }
            }
            ir::Callee::Method { receiver, method } => {
                let receiver = self.emit_operand(receiver)?;
                match (method.as_str(), args.as_slice()) {
                    ("toStr", []) => Some(format!("String.valueOf({receiver})")),
                    ("equals", [other]) => {
                        Some(format!("java.util.Objects.equals({receiver}, {other})"))
                    }
                    _ => Some(format!(
                        "{}.{}({})",
                        receiver,
                        java_member_name(method),
                        args.join(", ")
                    )),
                }
            }
            ir::Callee::Intrinsic(intrinsic) => self.emit_intrinsic(intrinsic, &args),
            ir::Callee::Named { path } => self.emit_named_runtime_call(path, &args),
            ir::Callee::Indirect(_) => None,
        }
    }

    fn emit_named_runtime_call(&self, path: &[String], args: &[String]) -> Option<String> {
        match path {
            [owner] if self.names.is_java_type(owner) => Some(format!(
                "new {}{}({})",
                self.names.named_type(owner),
                self.names.java_constructor_type_args(owner),
                args.join(", ")
            )),
            [owner, method] if self.names.is_java_type(owner) => Some(format!(
                "{}.{}({})",
                self.names.named_type(owner),
                java_member_name(method),
                args.join(", ")
            )),
            [owner, method] if owner == "Array" => {
                let target = match method.as_str() {
                    "ofInt" | "ofFloat" | "ofBool" | "ofStr" | "ofRune" | "fill" => {
                        format!("lume.runtime.LumeArray.{}", java_member_name(method))
                    }
                    _ => return None,
                };
                Some(format!("{target}({})", args.join(", ")))
            }
            _ => None,
        }
    }

    fn emit_intrinsic(&self, intrinsic: &ir::Intrinsic, args: &[String]) -> Option<String> {
        match intrinsic {
            ir::Intrinsic::Print => Some(format!(
                "lume.runtime.LumeRuntime.print({})",
                args.join(", ")
            )),
            ir::Intrinsic::Println => Some(format!(
                "lume.runtime.LumeRuntime.println({})",
                args.join(", ")
            )),
            ir::Intrinsic::Printf => Some(format!(
                "lume.runtime.LumeRuntime.printf({})",
                args.join(", ")
            )),
            ir::Intrinsic::Panic => Some(format!(
                "lume.runtime.LumePanic.panic({})",
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
                    "lume.runtime.LumeRuntime.assertTrue({condition}, {message})"
                ))
            }
            ir::Intrinsic::ListAppend => {
                if args.len() != 2 {
                    return None;
                }
                Some(format!("{}.add({})", args[0], args[1]))
            }
            ir::Intrinsic::IterInit
            | ir::Intrinsic::IterHasNext
            | ir::Intrinsic::IterNext
            | ir::Intrinsic::ExtractSuccessIsSet
            | ir::Intrinsic::ExtractSuccessValue
            | ir::Intrinsic::VariantIs(_)
            | ir::Intrinsic::VariantField(_) => None,
        }
    }

    fn emit_tuple(&self, items: &[ir::Operand]) -> Option<String> {
        if !(2..=8).contains(&items.len()) {
            return None;
        }
        let args = self.emit_operands(items)?;
        Some(format!(
            "new lume.runtime.Tuple{}<>({})",
            items.len(),
            args.join(", ")
        ))
    }

    fn emit_construct(&self, ty: &ir::Type, fields: &[ir::NamedOperand]) -> Option<String> {
        let ir::Type::Named { name, .. } = ty else {
            return None;
        };
        let args = fields
            .iter()
            .map(|field| self.emit_operand(&field.value))
            .collect::<Option<Vec<_>>>()?;
        Some(format!(
            "new {}({})",
            self.names.named_type(name),
            args.join(", ")
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
        Some(format!(
            "new {}.{}<>({})",
            java_type_name(enum_name),
            java_type_name(case_name),
            args.join(", ")
        ))
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
            ir::Place::Field { base, name } => Some(format!(
                "{}.{}",
                self.emit_operand(base)?,
                java_member_name(name)
            )),
            ir::Place::Index { .. } => None,
        }
    }

    fn place_type(&self, place: &ir::Place) -> Option<ir::Type> {
        match place {
            ir::Place::Local(id) => self.function.locals.get(id.0).map(|local| local.ty.clone()),
            ir::Place::Global(id) => self
                .bundle
                .ir
                .globals
                .get(id.0)
                .map(|global| global.ty.clone()),
            ir::Place::Field { .. } | ir::Place::Index { .. } => None,
        }
    }

    fn operand_type(&self, operand: &ir::Operand) -> Option<ir::Type> {
        match operand {
            ir::Operand::Copy(place) | ir::Operand::Move(place) => self.place_type(place),
            ir::Operand::Const(_) => None,
        }
    }

    fn rvalue_type(&self, value: &ir::RValue) -> Option<ir::Type> {
        match value {
            ir::RValue::Use(operand) => self.operand_type(operand),
            ir::RValue::Call { callee, .. } => match callee {
                ir::Callee::Direct(id) => self
                    .bundle
                    .ir
                    .function(*id)
                    .map(|function| function.return_ty.clone()),
                _ => None,
            },
            ir::RValue::Construct { ty, .. } => Some(ty.clone()),
            ir::RValue::Cast { ty, .. } => Some(ty.clone()),
            _ => None,
        }
    }

    fn coerce_to_target_type(
        &self,
        expr: String,
        source_ty: Option<ir::Type>,
        target_ty: &ir::Type,
    ) -> String {
        if is_named_builtin(target_ty, "Int32")
            && (source_ty.is_none() || source_is_wide_int(source_ty.as_ref()))
        {
            return format!("((int) ({expr}))");
        }
        if is_named_builtin(target_ty, "Float32")
            && (source_ty.is_none() || source_is_wide_float(source_ty.as_ref()))
        {
            return format!("((float) ({expr}))");
        }
        if !matches!(source_ty, Some(ir::Type::Unknown)) || matches!(target_ty, ir::Type::Unknown) {
            return expr;
        }
        if is_java_void_type(target_ty) {
            return expr;
        }
        format!("(({}) {expr})", self.names.value_type(target_ty))
    }

    fn local_reference(&self, local: &ir::Local) -> String {
        if matches!(local.kind, ir::LocalKind::Capture) && local.name == "this" {
            "this".to_string()
        } else {
            java_local_name(local)
        }
    }

    fn local_is_declared_elsewhere(&self, local: &ir::Local) -> bool {
        matches!(local.kind, ir::LocalKind::Param)
            || (matches!(local.kind, ir::LocalKind::Capture) && local.name == "this")
    }
}

fn rvalue_can_be_java_statement(value: &ir::RValue) -> bool {
    matches!(
        value,
        ir::RValue::Call { .. } | ir::RValue::Construct { .. } | ir::RValue::Variant { .. }
    )
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
    java_type_param_counts: HashMap<String, usize>,
}

impl JavaNames {
    fn from_bundle(bundle: &BackendBundle) -> Self {
        let java_types = bundle
            .descriptors
            .types
            .iter()
            .filter_map(|ty| match &ty.origin {
                DescriptorOrigin::Java { qualified_name } => {
                    Some((ty.name.clone(), qualified_name.clone()))
                }
                DescriptorOrigin::Lume => None,
            })
            .collect();
        let java_type_param_counts = bundle
            .descriptors
            .types
            .iter()
            .filter_map(|ty| match &ty.origin {
                DescriptorOrigin::Java { .. } => Some((ty.name.clone(), ty.type_params.len())),
                DescriptorOrigin::Lume => None,
            })
            .collect();
        Self {
            java_types,
            java_type_param_counts,
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
            ir::Type::Never => "lume.runtime.LumePanic".to_string(),
            ir::Type::Unit => "lume.runtime.LumeUnit".to_string(),
            ir::Type::Bool => "Boolean".to_string(),
            ir::Type::Int => "Long".to_string(),
            ir::Type::Float => "Double".to_string(),
            ir::Type::Str => "String".to_string(),
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
            ir::Type::Record(_) | ir::Type::Function { .. } => "Object".to_string(),
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
            ir::Type::Named { name, args } if name == "List" && args.len() == 1 => {
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

    fn builtin_container(&self, name: &str, args: &[ir::Type]) -> String {
        match name {
            "Array" if args.len() == 1 => {
                format!("lume.runtime.LumeArray<{}>", self.value_type(&args[0]))
            }
            "Either" if args.len() == 2 => format!(
                "lume.runtime.Either<{}, {}>",
                self.value_type(&args[0]),
                self.value_type(&args[1])
            ),
            "List" if args.len() == 1 => {
                format!("lume.runtime.LumeList<{}>", self.value_type(&args[0]))
            }
            "Map" if args.len() == 2 => format!(
                "lume.runtime.LumeMap<{}, {}>",
                self.value_type(&args[0]),
                self.value_type(&args[1])
            ),
            "Option" if args.len() == 1 => {
                format!("lume.runtime.Option<{}>", self.value_type(&args[0]))
            }
            "Result" if args.len() == 2 => format!(
                "lume.runtime.Result<{}, {}>",
                self.value_type(&args[0]),
                self.value_type(&args[1])
            ),
            "Set" if args.len() == 1 => {
                format!("lume.runtime.LumeSet<{}>", self.value_type(&args[0]))
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
        format!("lume.runtime.Tuple{}<{args}>", items.len())
    }
}

fn java_named_builtin_value(name: &str) -> Option<String> {
    match name {
        "Any" => Some("Object".to_string()),
        "Unit" => Some("lume.runtime.LumeUnit".to_string()),
        "Bool" => Some("Boolean".to_string()),
        "Int" => Some("Long".to_string()),
        "Int32" => Some("Integer".to_string()),
        "Float" => Some("Double".to_string()),
        "Float32" => Some("Float".to_string()),
        "Str" => Some("String".to_string()),
        "Rune" => Some("Integer".to_string()),
        _ => None,
    }
}

fn java_named_builtin_annotation(name: &str) -> Option<String> {
    match name {
        "Bool" => Some("boolean".to_string()),
        "Int" => Some("long".to_string()),
        "Int32" => Some("int".to_string()),
        "Float" => Some("double".to_string()),
        "Float32" => Some("float".to_string()),
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

fn java_local_name(local: &ir::Local) -> String {
    format!("{}_{}", java_member_name(&local.name), local.id.0)
}

fn java_default_value(ty: &ir::Type) -> String {
    if is_java_void_type(ty) {
        return "lume.runtime.LumeUnit.INSTANCE".to_string();
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

fn source_is_wide_int(ty: Option<&ir::Type>) -> bool {
    matches!(ty, Some(ir::Type::Int))
        || matches!(ty, Some(ir::Type::Named { name, args }) if name == "Int" && args.is_empty())
}

fn source_is_wide_float(ty: Option<&ir::Type>) -> bool {
    matches!(ty, Some(ir::Type::Float))
        || matches!(ty, Some(ir::Type::Named { name, args }) if name == "Float" && args.is_empty())
}

fn java_constant(constant: &ir::Constant) -> String {
    match constant {
        ir::Constant::Unit => "lume.runtime.LumeUnit.INSTANCE".to_string(),
        ir::Constant::Bool(value) => value.to_string(),
        ir::Constant::Int(value) => format!("{value}L"),
        ir::Constant::Float(value) => {
            let mut rendered = value.to_string();
            if !rendered.contains('.') && !rendered.contains('e') && !rendered.contains('E') {
                rendered.push_str(".0");
            }
            rendered
        }
        ir::Constant::String(value) => java_string_literal(&decode_lume_string_literal(value)),
        ir::Constant::List(items) => {
            let items = items
                .iter()
                .map(java_constant)
                .collect::<Vec<_>>()
                .join(", ");
            format!("lume.runtime.LumeList.of({items})")
        }
    }
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
