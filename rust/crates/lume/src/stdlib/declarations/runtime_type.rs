use super::super::schema::*;
use crate::ast::{Item, TypeKind, TypeMember};

pub(super) fn items() -> Vec<Item> {
    vec![
        Item::Type(declaration(
            "Annotated",
            TypeKind::Interface,
            vec![],
            vec![],
            methods(vec![
                generic_method(
                    "annotation",
                    vec![reified_param("Ann")],
                    vec![],
                    option(ty("AnnotationValue")),
                ),
                generic_method(
                    "hasAnnotation",
                    vec![reified_param("Ann")],
                    vec![],
                    ty("Bool"),
                ),
            ]),
        )),
        Item::Type(declaration(
            "AnnotationValue",
            TypeKind::Interface,
            vec![],
            vec![],
            methods(vec![
                getter("name", ty("Str")),
                method("field", vec![param("name", ty("Str"))], option(ty("Any"))),
                method("str", vec![param("name", ty("Str"))], option(ty("Str"))),
            ]),
        )),
        Item::Type(declaration(
            "ReflectionError",
            TypeKind::Class,
            vec![],
            vec![],
            vec![TypeMember::Field(field("message", ty("Str")))],
        )),
        Item::Type(declaration(
            "TypeKind",
            TypeKind::Enum,
            vec![],
            vec![],
            [
                "Class",
                "Shape",
                "Union",
                "Interface",
                "Object",
                "Annotation",
                "Primitive",
                "Tuple",
                "Function",
                "AnonymousShape",
            ]
            .into_iter()
            .map(|name| variant(name, vec![]))
            .collect(),
        )),
        Item::Type(declaration(
            "Type",
            TypeKind::Interface,
            vec![type_param("A")],
            vec![ty("Annotated")],
            methods(vec![
                getter("name", option(ty("Str"))),
                getter("qualifiedName", option(ty("Str"))),
                getter("kind", ty("TypeKind")),
                method(
                    "asClass",
                    vec![],
                    option(applied("ClassType", vec![ty("A")])),
                ),
                method(
                    "asShape",
                    vec![],
                    option(applied("ShapeType", vec![ty("A")])),
                ),
                method(
                    "asUnion",
                    vec![],
                    option(applied("UnionType", vec![ty("A")])),
                ),
                method(
                    "asInterface",
                    vec![],
                    option(applied("InterfaceType", vec![ty("A")])),
                ),
                method(
                    "asObject",
                    vec![],
                    option(applied("ObjectType", vec![ty("A")])),
                ),
                method(
                    "asAnnotation",
                    vec![],
                    option(applied("AnnotationType", vec![ty("A")])),
                ),
            ]),
        )),
        Item::Type(declaration(
            "ClassType",
            TypeKind::Interface,
            vec![type_param("A")],
            vec![applied("Type", vec![ty("A")])],
            methods(vec![
                getter("fields", vector(ty("Field"))),
                getter("methods", vector(ty("Method"))),
                method("field", vec![param("name", ty("Str"))], option(ty("Field"))),
                method(
                    "method",
                    vec![param("name", ty("Str"))],
                    option(ty("Method")),
                ),
                method(
                    "construct",
                    vec![vararg("args", ty("Any"))],
                    result(ty("A"), ty("ReflectionError")),
                ),
            ]),
        )),
        Item::Type(declaration(
            "ShapeType",
            TypeKind::Interface,
            vec![type_param("A")],
            vec![applied("Type", vec![ty("A")])],
            methods(vec![
                getter("fields", vector(ty("Field"))),
                getter("methods", vector(ty("Method"))),
                method(
                    "construct",
                    vec![vararg("args", ty("Any"))],
                    result(ty("A"), ty("ReflectionError")),
                ),
            ]),
        )),
        Item::Type(declaration(
            "UnionType",
            TypeKind::Interface,
            vec![type_param("A")],
            vec![applied("Type", vec![ty("A")])],
            methods(vec![
                getter("alternatives", vector(ty("UnionAlternative"))),
                method(
                    "alternative",
                    vec![param("name", ty("Str"))],
                    option(ty("UnionAlternative")),
                ),
                getter("methods", vector(ty("Method"))),
            ]),
        )),
        Item::Type(declaration(
            "InterfaceType",
            TypeKind::Interface,
            vec![type_param("A")],
            vec![applied("Type", vec![ty("A")])],
            methods(vec![getter("methods", vector(ty("Method")))]),
        )),
        Item::Type(declaration(
            "ObjectType",
            TypeKind::Interface,
            vec![type_param("A")],
            vec![applied("Type", vec![ty("A")])],
            methods(vec![
                getter("fields", vector(ty("Field"))),
                getter("methods", vector(ty("Method"))),
                method("field", vec![param("name", ty("Str"))], option(ty("Field"))),
                method(
                    "method",
                    vec![param("name", ty("Str"))],
                    option(ty("Method")),
                ),
            ]),
        )),
        Item::Type(declaration(
            "AnnotationType",
            TypeKind::Interface,
            vec![type_param("A")],
            vec![applied("Type", vec![ty("A")])],
            methods(vec![getter("fields", vector(ty("Field")))]),
        )),
        Item::Type(declaration(
            "Field",
            TypeKind::Interface,
            vec![],
            vec![ty("Annotated")],
            methods(vec![
                getter("name", ty("Str")),
                getter("fieldType", applied("Type", vec![wildcard()])),
                getter("isPrivate", ty("Bool")),
                method(
                    "get",
                    vec![param("receiver", ty("Any"))],
                    result(ty("Any"), ty("ReflectionError")),
                ),
            ]),
        )),
        Item::Type(declaration(
            "Method",
            TypeKind::Interface,
            vec![],
            vec![ty("Annotated")],
            methods(vec![
                getter("name", ty("Str")),
                getter("params", vector(ty("Param"))),
                getter("returnType", applied("Type", vec![wildcard()])),
                method(
                    "call",
                    vec![param("receiver", ty("Any")), vararg("args", ty("Any"))],
                    result(ty("Any"), ty("ReflectionError")),
                ),
            ]),
        )),
        Item::Type(declaration(
            "Param",
            TypeKind::Interface,
            vec![],
            vec![],
            methods(vec![
                getter("name", ty("Str")),
                getter("paramType", applied("Type", vec![wildcard()])),
            ]),
        )),
        Item::Type(declaration(
            "UnionAlternative",
            TypeKind::Interface,
            vec![],
            vec![ty("Annotated")],
            methods(vec![
                getter("name", ty("Str")),
                getter("fields", vector(ty("Field"))),
                method(
                    "construct",
                    vec![vararg("args", ty("Any"))],
                    result(ty("Any"), ty("ReflectionError")),
                ),
            ]),
        )),
    ]
}
