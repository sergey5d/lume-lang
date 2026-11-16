use super::super::schema::*;
use crate::ast::{Item, TypeKind};

pub(super) fn items() -> Vec<Item> {
    vec![
        Item::Type(declaration(
            "Array",
            TypeKind::Class,
            vec![type_param("T")],
            vec![applied("Iterable", vec![ty("T")])],
            methods(vec![
                constructor(vec![vararg("values", ty("T"))]),
                method("at", vec![param("index", ty("Int"))], option(ty("T"))),
                method(
                    "setAt",
                    vec![param("index", ty("Int")), param("value", ty("T"))],
                    result(ty("T"), ty("InvalidIndex")),
                ),
                getter("first", option(ty("T"))),
                getter("last", option(ty("T"))),
                method("clone", vec![], applied("Array", vec![ty("T")])),
                generic_method(
                    "map",
                    vec![type_param("X")],
                    vec![param("f", callback(vec![ty("T")], ty("X")))],
                    applied("Array", vec![ty("X")]),
                ),
                method(
                    "forEach",
                    vec![param("f", callback(vec![ty("T")], ty("Unit")))],
                    ty("Unit"),
                ),
                method(
                    "count",
                    vec![param("f", callback(vec![ty("T")], ty("Bool")))],
                    ty("Int"),
                ),
                method(
                    "exists",
                    vec![param("f", callback(vec![ty("T")], ty("Bool")))],
                    ty("Bool"),
                ),
                method(
                    "forAll",
                    vec![param("f", callback(vec![ty("T")], ty("Bool")))],
                    ty("Bool"),
                ),
                method("contains", vec![param("e", ty("T"))], ty("Bool")),
                method("find", vec![param("e", ty("T"))], option(ty("T"))),
                method("indexOf", vec![param("value", ty("T"))], option(ty("Int"))),
                generic_method(
                    "zip",
                    vec![type_param("X")],
                    vec![param("other", applied("Array", vec![ty("X")]))],
                    applied("Array", vec![tuple(vec![ty("T"), ty("X")])]),
                ),
                method(
                    "zipWithIndex",
                    vec![],
                    applied("Array", vec![tuple(vec![ty("T"), ty("Int")])]),
                ),
                getter("size", ty("Int")),
                method("iterator", vec![], applied("Iterator", vec![ty("T")])),
            ]),
        )),
        Item::Type(declaration(
            "Array",
            TypeKind::Object,
            vec![],
            vec![],
            methods(vec![
                generic_method(
                    "fill",
                    vec![type_param("T")],
                    vec![param("length", ty("Int")), param("value", ty("T"))],
                    applied("Array", vec![ty("T")]),
                ),
                generic_method(
                    "generate",
                    vec![type_param("T")],
                    vec![
                        param("length", ty("Int")),
                        param("generator", callback(vec![ty("Int")], ty("T"))),
                    ],
                    applied("Array", vec![ty("T")]),
                ),
                generic_method(
                    "from",
                    vec![type_param("T")],
                    vec![param("values", applied("Iterable", vec![ty("T")]))],
                    applied("Array", vec![ty("T")]),
                ),
            ]),
        )),
    ]
}
