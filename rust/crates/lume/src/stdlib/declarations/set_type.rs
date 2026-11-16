use super::super::schema::*;
use crate::ast::{Item, TypeKind};

pub(super) fn items() -> Vec<Item> {
    let set = || applied("Set", vec![ty("T")]);
    vec![
        Item::Type(declaration(
            "Set",
            TypeKind::Class,
            vec![type_param("T")],
            vec![applied("Iterable", vec![ty("T")])],
            methods(vec![
                constructor(vec![vararg("values", ty("T"))]),
                method("add", vec![param("value", ty("T"))], set()),
                method(
                    "addAll",
                    vec![param("values", applied("Iterable", vec![ty("T")]))],
                    set(),
                ),
                method("iterator", vec![], applied("Iterator", vec![ty("T")])),
                generic_method(
                    "map",
                    vec![type_param("X")],
                    vec![param("f", callback(vec![ty("T")], ty("X")))],
                    applied("Set", vec![ty("X")]),
                ),
                generic_method(
                    "flatMap",
                    vec![type_param("X")],
                    vec![param(
                        "f",
                        callback(vec![ty("T")], applied("Iterable", vec![ty("X")])),
                    )],
                    applied("Set", vec![ty("X")]),
                ),
                method(
                    "filter",
                    vec![param("f", callback(vec![ty("T")], ty("Bool")))],
                    set(),
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
                method(
                    "forEach",
                    vec![param("f", callback(vec![ty("T")], ty("Unit")))],
                    ty("Unit"),
                ),
                method("contains", vec![param("value", ty("T"))], ty("Bool")),
                getter("isEmpty", ty("Bool")),
                getter("nonEmpty", ty("Bool")),
                getter("size", ty("Int")),
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
        )),
        Item::Type(declaration(
            "Set",
            TypeKind::Object,
            vec![],
            vec![],
            methods(vec![generic_method(
                "from",
                vec![type_param("T")],
                vec![param("values", applied("Iterable", vec![ty("T")]))],
                applied("Set", vec![ty("T")]),
            )]),
        )),
    ]
}
