use super::super::schema::*;
use crate::ast::{Item, TypeKind};

pub(super) fn items() -> Vec<Item> {
    let list = || applied("LinkedList", vec![ty("T")]);
    vec![Item::Type(declaration(
        "LinkedList",
        TypeKind::Class,
        vec![type_param("T")],
        vec![applied("Iterable", vec![ty("T")])],
        methods(vec![
            constructor(vec![vararg("values", ty("T"))]),
            method("add", vec![param("value", ty("T"))], list()),
            method(
                "addAll",
                vec![param("values", applied("Iterable", vec![ty("T")]))],
                list(),
            ),
            generic_method(
                "map",
                vec![type_param("X")],
                vec![param("f", callback(vec![ty("T")], ty("X")))],
                applied("LinkedList", vec![ty("X")]),
            ),
            generic_method(
                "flatMap",
                vec![type_param("X")],
                vec![param(
                    "f",
                    callback(vec![ty("T")], applied("Iterable", vec![ty("X")])),
                )],
                applied("LinkedList", vec![ty("X")]),
            ),
            method(
                "filter",
                vec![param("f", callback(vec![ty("T")], ty("Bool")))],
                list(),
            ),
            generic_method(
                "fold",
                vec![type_param("X")],
                vec![
                    param("initial", ty("X")),
                    param("f", callback(vec![ty("X"), ty("T")], ty("X"))),
                ],
                ty("X"),
            ),
            method(
                "reduce",
                vec![param("f", callback(vec![ty("T"), ty("T")], ty("T")))],
                option(ty("T")),
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
            method("indexOf", vec![param("value", ty("T"))], option(ty("Int"))),
            generic_method(
                "zip",
                vec![type_param("X")],
                vec![param("other", applied("LinkedList", vec![ty("X")]))],
                applied("LinkedList", vec![tuple(vec![ty("T"), ty("X")])]),
            ),
            method(
                "zipWithIndex",
                vec![],
                applied("LinkedList", vec![tuple(vec![ty("T"), ty("Int")])]),
            ),
            method("at", vec![param("index", ty("Int"))], option(ty("T"))),
            method(
                "setAt",
                vec![param("index", ty("Int")), param("value", ty("T"))],
                result(ty("T"), ty("InvalidIndex")),
            ),
            method(
                "insertAt",
                vec![param("index", ty("Int")), param("value", ty("T"))],
                result(ty("Unit"), ty("InvalidIndex")),
            ),
            method(
                "removeAt",
                vec![param("index", ty("Int"))],
                result(ty("T"), ty("InvalidIndex")),
            ),
            getter("first", option(ty("T"))),
            getter("last", option(ty("T"))),
            method("tail", vec![], list()),
            getter("isEmpty", ty("Bool")),
            getter("nonEmpty", ty("Bool")),
            method("removeFirst", vec![], option(ty("T"))),
            method("removeLast", vec![], option(ty("T"))),
            getter("size", ty("Int")),
            method(
                "forEach",
                vec![param("f", callback(vec![ty("T")], ty("Unit")))],
                ty("Unit"),
            ),
            method("iterator", vec![], applied("Iterator", vec![ty("T")])),
            method("makeStr", vec![param("separator", ty("Str"))], ty("Str")),
            method(
                "makeStr",
                vec![
                    param("separator", ty("Str")),
                    param("render", callback(vec![ty("T")], ty("Str"))),
                ],
                ty("Str"),
            ),
        ]),
    ))]
}
