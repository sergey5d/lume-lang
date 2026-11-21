//! Standard-library contracts and native implementation bindings.
//!
//! Contracts are authored in Rust and normalized to the same declaration model
//! as source classes and foreign stubs. They do not depend on finding .lum files.

mod declarations;
pub(crate) mod native;
mod operators;
mod schema;

#[cfg(test)]
mod tests;

use std::{collections::HashMap, sync::OnceLock};

use crate::{
    ast::{self, BinaryOp, TypeDecl, TypeKind, TypeMember, TypeRef, UnaryOp},
    ir,
};

pub(crate) fn declarations() -> &'static ast::Program {
    static PROGRAM: OnceLock<ast::Program> = OnceLock::new();
    PROGRAM.get_or_init(declarations::program)
}

pub(crate) fn module_declarations(path: &str) -> Option<&'static ast::Program> {
    static JSON: OnceLock<ast::Program> = OnceLock::new();
    static REGEX: OnceLock<ast::Program> = OnceLock::new();
    let slot = match path {
        "lume/json" => &JSON,
        "lume/regex" => &REGEX,
        _ => return None,
    };
    Some(slot.get_or_init(|| declarations::module_program(path).expect("registered native module")))
}

fn declaration_programs() -> impl Iterator<Item = &'static ast::Program> {
    std::iter::once(declarations()).chain(
        ["lume/json", "lume/regex"]
            .into_iter()
            .filter_map(module_declarations),
    )
}

pub(crate) fn type_declaration(name: &str, kind: TypeKind) -> Option<&'static TypeDecl> {
    declaration_programs()
        .flat_map(|program| &program.items)
        .find_map(|item| match item {
            ast::Item::Type(ty) if ty.name == name && ty.kind == kind => Some(ty),
            _ => None,
        })
}

pub(crate) fn is_primitive(name: &str) -> bool {
    descriptor(name).is_some_and(|descriptor| descriptor.representation.is_some())
}

pub use operators::NativeRepresentation;

#[derive(Debug, Clone)]
pub struct TypeDescriptor {
    pub name: String,
    pub kind: TypeKind,
    pub declaration: TypeDecl,
    pub primitive: bool,
    pub representation: Option<NativeRepresentation>,
    pub operators: Vec<OperatorDescriptor>,
    pub host_capabilities: Vec<HostCapability>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostCapability {
    NativeFiles,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperatorDescriptor {
    Binary(BinaryOp),
    Unary(UnaryOp),
    Index,
    IndexAssign,
    Iterate,
    Extract,
}

pub(crate) fn descriptors() -> &'static [TypeDescriptor] {
    static TYPES: OnceLock<Vec<TypeDescriptor>> = OnceLock::new();
    TYPES.get_or_init(|| {
        declaration_programs()
            .flat_map(|program| &program.items)
            .filter_map(|item| {
                let ast::Item::Type(declaration) = item else {
                    return None;
                };
                Some(TypeDescriptor {
                    name: declaration.name.clone(),
                    kind: declaration.kind,
                    primitive: declaration.kind != TypeKind::Object
                        && operators::capabilities(&declaration.name)
                            .is_some_and(|entry| entry.representation.is_some()),
                    representation: (declaration.kind != TypeKind::Object)
                        .then(|| {
                            operators::capabilities(&declaration.name)
                                .and_then(|entry| entry.representation)
                        })
                        .flatten(),
                    declaration: declaration.clone(),
                    operators: operator_capabilities(&declaration.name, declaration.kind),
                    host_capabilities: if matches!(
                        declaration.name.as_str(),
                        "File" | "FileStream" | "TextFileReader"
                    ) {
                        vec![HostCapability::NativeFiles]
                    } else {
                        Vec::new()
                    },
                })
            })
            .collect()
    })
}

pub(crate) fn descriptor(name: &str) -> Option<&'static TypeDescriptor> {
    descriptors()
        .iter()
        .find(|descriptor| descriptor.name == name && descriptor.kind != TypeKind::Object)
}

pub(crate) fn supports_operator(name: &str, op: BinaryOp) -> bool {
    descriptor(name).is_some_and(|descriptor| {
        descriptor
            .operators
            .contains(&OperatorDescriptor::Binary(op))
    })
}

pub(crate) fn supports_unary_operator(name: &str, op: UnaryOp) -> bool {
    descriptor(name).is_some_and(|descriptor| {
        descriptor
            .operators
            .contains(&OperatorDescriptor::Unary(op))
    })
}

fn operator_capabilities(name: &str, kind: TypeKind) -> Vec<OperatorDescriptor> {
    if kind == TypeKind::Object {
        return Vec::new();
    }
    operators::capabilities(name)
        .map(|entry| entry.operations.to_vec())
        .unwrap_or_default()
}

/// A member contract with the receiver's arguments substituted. Unbound method
/// parameters are holes in this backend view, not in the canonical contract.
#[derive(Debug, Clone)]
pub(crate) struct NativeSignature {
    pub param_names: Vec<String>,
    pub params: Vec<ir::Type>,
    pub return_ty: ir::Type,
    pub getter: bool,
    pub variadic: bool,
    pub generic_conditions: Vec<ir::GenericCondition>,
}

pub(crate) fn method_signature(
    receiver: &ir::Type,
    member: &str,
    arity: usize,
) -> Option<NativeSignature> {
    method_contract(receiver, member, arity).map(erase_unbound_parameters)
}

pub(crate) fn method_contract(
    receiver: &ir::Type,
    member: &str,
    arity: usize,
) -> Option<NativeSignature> {
    let (owner, args) = receiver_type(receiver)?;
    let declaration = &descriptor(owner)?.declaration;
    instantiate_signature(declaration, &args, member, arity)
}

pub(crate) fn static_method_signature(
    owner: &str,
    member: &str,
    arity: usize,
) -> Option<NativeSignature> {
    static_method_contract(owner, member, arity).map(erase_unbound_parameters)
}

pub(crate) fn static_method_contract(
    owner: &str,
    member: &str,
    arity: usize,
) -> Option<NativeSignature> {
    instantiate_signature(
        type_declaration(owner, TypeKind::Object)?,
        &[],
        member,
        arity,
    )
}

fn erase_unbound_parameters(mut signature: NativeSignature) -> NativeSignature {
    let empty = HashMap::new();
    signature.params = signature
        .params
        .into_iter()
        .map(|ty| substitute_ir(ty, &empty, false))
        .collect();
    signature.return_ty = substitute_ir(signature.return_ty, &empty, false);
    signature
}

fn receiver_type(receiver: &ir::Type) -> Option<(&str, Vec<ir::Type>)> {
    Some(match receiver {
        ir::Type::Bool => ("Bool", Vec::new()),
        ir::Type::Int => ("Int", Vec::new()),
        ir::Type::Float => ("Float", Vec::new()),
        ir::Type::Str => ("Str", Vec::new()),
        ir::Type::Unit => ("Unit", Vec::new()),
        ir::Type::Never => ("Never", Vec::new()),
        ir::Type::Named { name, args } => (name.as_str(), args.clone()),
        _ => return None,
    })
}

fn instantiate_signature(
    declaration: &TypeDecl,
    args: &[ir::Type],
    member: &str,
    arity: usize,
) -> Option<NativeSignature> {
    let candidates = member_declarations(declaration, member);
    let method = candidates.iter().find(|method| {
        let minimum = method
            .params
            .iter()
            .filter(|param| !param.variadic && param.initializer.is_none())
            .count();
        arity >= minimum
            && (arity <= method.params.len()
                || method.params.last().is_some_and(|param| param.variadic))
    })?;
    let variables = declaration
        .type_params
        .iter()
        .chain(&method.type_params)
        .map(|param| param.name.as_str())
        .collect::<Vec<_>>();
    let subst = declaration
        .type_params
        .iter()
        .enumerate()
        .map(|(index, param)| {
            (
                param.name.clone(),
                args.get(index).cloned().unwrap_or(ir::Type::Unknown),
            )
        })
        .collect::<HashMap<_, _>>();
    let mut generic_conditions = declaration
        .type_conditions
        .iter()
        .chain(&method.type_conditions)
        .map(|condition| match condition {
            ast::GenericCondition::Bound { subject, bound, .. } => ir::GenericCondition::Bound {
                subject: substitute_ir(type_to_ir(subject, &variables), &subst, true),
                bound: substitute_ir(type_to_ir(bound, &variables), &subst, true),
            },
            ast::GenericCondition::Equal { left, right, .. } => ir::GenericCondition::Equal {
                left: substitute_ir(type_to_ir(left, &variables), &subst, true),
                right: substitute_ir(type_to_ir(right, &variables), &subst, true),
            },
        })
        .collect::<Vec<_>>();
    for param in declaration.type_params.iter().chain(&method.type_params) {
        for bound in &param.bounds {
            generic_conditions.push(ir::GenericCondition::Bound {
                subject: substitute_ir(ir::Type::TypeParam(param.name.clone()), &subst, true),
                bound: substitute_ir(type_to_ir(bound, &variables), &subst, true),
            });
        }
    }
    Some(NativeSignature {
        param_names: method
            .params
            .iter()
            .map(|param| param.name.clone())
            .collect(),
        params: method
            .params
            .iter()
            .map(|param| {
                substitute_ir(
                    type_to_ir(
                        param.ty.as_ref().expect("library parameters have types"),
                        &variables,
                    ),
                    &subst,
                    true,
                )
            })
            .collect(),
        return_ty: method
            .return_type
            .as_ref()
            .map(|ty| substitute_ir(type_to_ir(ty, &variables), &subst, true))
            .unwrap_or(ir::Type::Unit),
        getter: method.getter,
        variadic: method.params.last().is_some_and(|param| param.variadic),
        generic_conditions,
    })
}

fn substitute_ir(
    ty: ir::Type,
    subst: &HashMap<String, ir::Type>,
    preserve_unbound: bool,
) -> ir::Type {
    match ty {
        ir::Type::TypeParam(name) => subst.get(&name).cloned().unwrap_or_else(|| {
            if preserve_unbound {
                ir::Type::TypeParam(name)
            } else {
                ir::Type::Unknown
            }
        }),
        ir::Type::Named { name, args } => ir::Type::Named {
            name,
            args: args
                .into_iter()
                .map(|arg| substitute_ir(arg, subst, preserve_unbound))
                .collect(),
        },
        ir::Type::Function { params, ret } => ir::Type::Function {
            params: params
                .into_iter()
                .map(|param| substitute_ir(param, subst, preserve_unbound))
                .collect(),
            ret: Box::new(substitute_ir(*ret, subst, preserve_unbound)),
        },
        ir::Type::Tuple(items) => ir::Type::Tuple(
            items
                .into_iter()
                .map(|item| substitute_ir(item, subst, preserve_unbound))
                .collect(),
        ),
        ir::Type::Union(items) => ir::Type::Union(
            items
                .into_iter()
                .map(|item| substitute_ir(item, subst, preserve_unbound))
                .collect(),
        ),
        ir::Type::Record(fields) => ir::Type::Record(
            fields
                .into_iter()
                .map(|field| ir::NamedType {
                    name: field.name,
                    ty: substitute_ir(field.ty, subst, preserve_unbound),
                })
                .collect(),
        ),
        other => other,
    }
}

pub(crate) fn runtime_method_signature(
    owner: &str,
    member: &str,
    arity: usize,
) -> Option<(Vec<String>, Vec<ir::Type>, ir::Type)> {
    let kind = descriptors()
        .iter()
        .find(|ty| ty.name == owner && ty.kind != TypeKind::Object)?
        .kind;
    let declaration = type_declaration(owner, kind)?;
    let methods = member_declarations(declaration, member);
    let method = methods.iter().find(|method| {
        method.params.len() == arity || method.params.last().is_some_and(|param| param.variadic)
    })?;
    let variables = declaration
        .type_params
        .iter()
        .chain(&method.type_params)
        .map(|param| param.name.as_str())
        .collect::<Vec<_>>();
    let names = method
        .params
        .iter()
        .map(|param| param.name.clone())
        .collect();
    let params = method
        .params
        .iter()
        .map(|param| {
            type_to_ir(
                param.ty.as_ref().expect("native parameters have types"),
                &variables,
            )
        })
        .collect();
    let ret = method
        .return_type
        .as_ref()
        .map(|ty| type_to_ir(ty, &variables))
        .unwrap_or(ir::Type::Unit);
    Some((names, params, ret))
}

pub(crate) fn member_declarations(declaration: &TypeDecl, name: &str) -> Vec<ast::MethodDecl> {
    fn collect(declaration: &TypeDecl, name: &str, seen: &mut Vec<String>) -> Vec<ast::MethodDecl> {
        if seen.contains(&declaration.name) {
            return Vec::new();
        }
        seen.push(declaration.name.clone());
        let mut methods = declaration
            .members
            .iter()
            .filter_map(|member| match member {
                TypeMember::Method(method) if method.name == name => Some(method.clone()),
                _ => None,
            })
            .collect::<Vec<_>>();
        if declaration.kind != TypeKind::Object {
            for extension in declaration_programs()
                .flat_map(|program| &program.items)
                .filter_map(|item| match item {
                    ast::Item::Extension(extension) => Some(extension),
                    _ => None,
                })
            {
                let TypeRef::Named {
                    name: target, args, ..
                } = &extension.target
                else {
                    continue;
                };
                if target != &declaration.name {
                    continue;
                }
                let substitutions = args
                    .iter()
                    .zip(&declaration.type_params)
                    .filter_map(|(argument, param)| match argument {
                        TypeRef::Named { name, args, .. } if args.is_empty() => {
                            Some((name.clone(), schema::ty(&param.name)))
                        }
                        _ => None,
                    })
                    .collect::<HashMap<_, _>>();
                for method in extension
                    .methods
                    .iter()
                    .filter(|method| method.name == name)
                {
                    let mut method = method.clone();
                    for param in &mut method.params {
                        if let Some(ty) = &mut param.ty {
                            substitute_ref(ty, &substitutions);
                        }
                    }
                    if let Some(ret) = &mut method.return_type {
                        substitute_ref(ret, &substitutions);
                    }
                    methods.push(method);
                }
            }
        }
        if !methods.is_empty() {
            return methods;
        }
        for bound in &declaration.with_bounds {
            let TypeRef::Named {
                name: owner, args, ..
            } = bound
            else {
                continue;
            };
            let Some(parent) = descriptors()
                .iter()
                .find(|ty| ty.name == *owner && ty.kind != TypeKind::Object)
            else {
                continue;
            };
            let substitutions = parent
                .declaration
                .type_params
                .iter()
                .zip(args)
                .map(|(param, arg)| (param.name.clone(), arg.clone()))
                .collect::<HashMap<_, _>>();
            for mut method in collect(&parent.declaration, name, seen) {
                for param in &mut method.params {
                    if let Some(ty) = &mut param.ty {
                        substitute_ref(ty, &substitutions);
                    }
                }
                if let Some(ret) = &mut method.return_type {
                    substitute_ref(ret, &substitutions);
                }
                methods.push(method);
            }
        }
        methods
    }
    collect(declaration, name, &mut Vec::new())
}

fn substitute_ref(ty: &mut TypeRef, substitutions: &HashMap<String, TypeRef>) {
    match ty {
        TypeRef::Named { name, args, .. } => {
            if args.is_empty()
                && let Some(replacement) = substitutions.get(name)
            {
                *ty = replacement.clone();
                return;
            }
            for arg in args {
                substitute_ref(arg, substitutions);
            }
        }
        TypeRef::Function { params, ret, .. } => {
            for param in params {
                substitute_ref(param, substitutions);
            }
            substitute_ref(ret, substitutions);
        }
        TypeRef::Tuple { fields, .. } => {
            for field in fields {
                substitute_ref(&mut field.ty, substitutions);
            }
        }
        TypeRef::Record { fields, .. } => {
            for field in fields {
                substitute_ref(&mut field.ty, substitutions);
            }
        }
        TypeRef::Union { members, .. } => {
            for member in members {
                substitute_ref(member, substitutions);
            }
        }
        TypeRef::Wildcard { .. } => {}
    }
}

pub(crate) fn type_to_ir(ty: &TypeRef, variables: &[&str]) -> ir::Type {
    if let TypeRef::Named { name, args, .. } = ty {
        if !variables.contains(&name.as_str()) {
            if let Some(alias) = declaration_programs()
                .flat_map(|program| &program.items)
                .find_map(|item| match item {
                    ast::Item::TypeAlias(alias)
                        if alias.name == *name && alias.type_params.len() == args.len() =>
                    {
                        Some(alias)
                    }
                    _ => None,
                })
            {
                let substitutions = alias
                    .type_params
                    .iter()
                    .zip(args)
                    .map(|(param, arg)| (param.name.clone(), arg.clone()))
                    .collect();
                let mut target = alias.target.clone();
                substitute_ref(&mut target, &substitutions);
                return type_to_ir(&target, variables);
            }
        }
    }
    match ty {
        TypeRef::Named { name, args, .. }
            if args.is_empty() && variables.contains(&name.as_str()) =>
        {
            ir::Type::TypeParam(name.clone())
        }
        TypeRef::Named { name, args, .. } if args.is_empty() => match name.as_str() {
            "Unit" => ir::Type::Unit,
            "Never" => ir::Type::Never,
            "Bool" => ir::Type::Bool,
            "Int" => ir::Type::Int,
            "Float" => ir::Type::Float,
            "Str" => ir::Type::Str,
            _ => ir::Type::Named {
                name: name.clone(),
                args: Vec::new(),
            },
        },
        TypeRef::Named { name, args, .. } => ir::Type::Named {
            name: name.clone(),
            args: args.iter().map(|arg| type_to_ir(arg, variables)).collect(),
        },
        TypeRef::Tuple { fields, .. } => ir::Type::Tuple(
            fields
                .iter()
                .map(|field| type_to_ir(&field.ty, variables))
                .collect(),
        ),
        TypeRef::Function { params, ret, .. } => ir::Type::Function {
            params: params
                .iter()
                .map(|param| type_to_ir(param, variables))
                .collect(),
            ret: Box::new(type_to_ir(ret, variables)),
        },
        TypeRef::Record { fields, .. } => ir::Type::Record(
            fields
                .iter()
                .map(|field| ir::NamedType {
                    name: field.name.clone(),
                    ty: type_to_ir(&field.ty, variables),
                })
                .collect(),
        ),
        TypeRef::Union { members, .. } => ir::Type::Union(
            members
                .iter()
                .map(|member| type_to_ir(member, variables))
                .collect(),
        ),
        TypeRef::Wildcard { .. } => ir::Type::Named {
            name: "Any".into(),
            args: Vec::new(),
        },
    }
}
