//! Builders for native library contracts using the ordinary declaration model.

use crate::{
    ast::{
        self, CallableBody, EnumCaseDecl, Expr, FieldDecl, FunctionDecl, GenericCondition, Item,
        MethodDecl, Param, TupleTypeField, TypeAliasDecl, TypeDecl, TypeKind, TypeMember,
        TypeParam, TypeRef, Visibility,
    },
    source::{LineColumn, Span},
};

pub(crate) fn span() -> Span {
    Span::new(0, 0, LineColumn::new(1, 1), LineColumn::new(1, 1))
}

pub(crate) fn ty(name: &str) -> TypeRef {
    applied(name, Vec::new())
}

pub(crate) fn applied(name: &str, args: Vec<TypeRef>) -> TypeRef {
    TypeRef::Named {
        name: name.into(),
        args,
        span: span(),
    }
}

pub(crate) fn wildcard() -> TypeRef {
    TypeRef::Wildcard { span: span() }
}

pub(crate) fn vector(item: TypeRef) -> TypeRef {
    applied("Vector", vec![item])
}

pub(crate) fn option(item: TypeRef) -> TypeRef {
    applied("Option", vec![item])
}

pub(crate) fn result(value: TypeRef, error: TypeRef) -> TypeRef {
    applied("Result", vec![value, error])
}

pub(crate) fn tuple(items: Vec<TypeRef>) -> TypeRef {
    TypeRef::Tuple {
        fields: items
            .into_iter()
            .map(|ty| TupleTypeField { ty, span: span() })
            .collect(),
        span: span(),
    }
}

pub(crate) fn callback(params: Vec<TypeRef>, ret: TypeRef) -> TypeRef {
    TypeRef::Function {
        params,
        ret: Box::new(ret),
        span: span(),
    }
}

pub(crate) fn type_param(name: &str) -> TypeParam {
    TypeParam {
        name: name.into(),
        reified: false,
        bounds: Vec::new(),
        span: span(),
    }
}

pub(crate) fn bounded_param(name: &str, bounds: Vec<TypeRef>) -> TypeParam {
    TypeParam {
        bounds,
        ..type_param(name)
    }
}

pub(crate) fn reified_param(name: &str) -> TypeParam {
    TypeParam {
        reified: true,
        ..type_param(name)
    }
}

pub(crate) fn bound(subject: TypeRef, bound: TypeRef) -> GenericCondition {
    GenericCondition::Bound {
        subject,
        bound,
        span: span(),
    }
}

pub(crate) fn equal(left: TypeRef, right: TypeRef) -> GenericCondition {
    GenericCondition::Equal {
        left,
        right,
        span: span(),
    }
}

pub(crate) fn param(name: &str, ty: TypeRef) -> Param {
    Param {
        name: name.into(),
        ty: Some(ty),
        initializer: None,
        variadic: false,
        span: span(),
    }
}

pub(crate) fn vararg(name: &str, item: TypeRef) -> Param {
    Param {
        variadic: true,
        ..param(name, vector(item))
    }
}

pub(crate) fn default_param(name: &str, ty: TypeRef, value: Expr) -> Param {
    Param {
        initializer: Some(value),
        ..param(name, ty)
    }
}

pub(crate) fn integer(value: i64) -> Expr {
    Expr::Integer {
        raw: value.to_string(),
        span: span(),
    }
}

pub(crate) fn string(value: &str) -> Expr {
    Expr::String {
        raw: format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\"")),
        span: span(),
    }
}

/// Native bodies are binding markers, never source implementations to lower.
pub(crate) fn native_body() -> CallableBody {
    CallableBody::Expr(Expr::Unit { span: span() })
}

pub(crate) fn method(name: &str, params: Vec<Param>, ret: TypeRef) -> MethodDecl {
    MethodDecl {
        annotations: Vec::new(),
        visibility: Visibility::Default,
        name: name.into(),
        getter: false,
        type_params: Vec::new(),
        type_conditions: Vec::new(),
        params,
        return_type: Some(ret),
        body: Some(native_body()),
        span: span(),
    }
}

pub(crate) fn generic_method(
    name: &str,
    types: Vec<TypeParam>,
    params: Vec<Param>,
    ret: TypeRef,
) -> MethodDecl {
    MethodDecl {
        type_params: types,
        ..method(name, params, ret)
    }
}

pub(crate) fn getter(name: &str, ret: TypeRef) -> MethodDecl {
    MethodDecl {
        getter: true,
        ..method(name, Vec::new(), ret)
    }
}

pub(crate) fn constructor(params: Vec<Param>) -> MethodDecl {
    MethodDecl {
        return_type: None,
        ..method("new", params, ty("Unit"))
    }
}

pub(crate) fn field(name: &str, ty: TypeRef) -> FieldDecl {
    FieldDecl {
        annotations: Vec::new(),
        visibility: Visibility::Default,
        mutable: false,
        from_mixin: false,
        name: name.into(),
        ty: Some(ty),
        initializer: None,
        span: span(),
    }
}

pub(crate) fn variant(name: &str, fields: Vec<FieldDecl>) -> TypeMember {
    TypeMember::Case(EnumCaseDecl {
        annotations: Vec::new(),
        kind: if fields.is_empty() {
            TypeKind::Object
        } else {
            TypeKind::Class
        },
        name: name.into(),
        fields,
        span: span(),
    })
}

pub(crate) fn declaration(
    name: &str,
    kind: TypeKind,
    types: Vec<TypeParam>,
    bounds: Vec<TypeRef>,
    mut members: Vec<TypeMember>,
) -> TypeDecl {
    if kind == TypeKind::Interface {
        for member in &mut members {
            if let TypeMember::Method(method) = member {
                method.body = None;
            }
        }
    }
    TypeDecl {
        annotations: Vec::new(),
        visibility: Visibility::Default,
        kind,
        name: name.into(),
        type_params: types,
        type_conditions: Vec::new(),
        with_bounds: bounds,
        mixins: Vec::new(),
        mixin_initializers: Vec::new(),
        members,
        span: span(),
    }
}

pub(crate) fn methods(items: Vec<MethodDecl>) -> Vec<TypeMember> {
    items.into_iter().map(TypeMember::Method).collect()
}

pub(crate) fn function(
    name: &str,
    types: Vec<TypeParam>,
    params: Vec<Param>,
    ret: TypeRef,
) -> Item {
    Item::Function(FunctionDecl {
        annotations: Vec::new(),
        visibility: Visibility::Default,
        name: name.into(),
        type_params: types,
        type_conditions: Vec::new(),
        params,
        return_type: Some(ret),
        body: native_body(),
        span: span(),
    })
}

pub(crate) fn alias(name: &str, target: TypeRef) -> Item {
    Item::TypeAlias(TypeAliasDecl {
        visibility: Visibility::Default,
        name: name.into(),
        type_params: Vec::new(),
        type_conditions: Vec::new(),
        target,
        span: span(),
    })
}

pub(crate) fn program(items: Vec<Item>) -> ast::Program {
    ast::Program {
        items,
        ..ast::Program::default()
    }
}
