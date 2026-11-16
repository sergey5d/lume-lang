use super::super::schema::*;
use crate::ast::{Item, TypeKind};

pub(super) fn items() -> Vec<Item> {
    let entry = || tuple(vec![ty("K"), ty("V")]);
    let map = || applied("Map", vec![ty("K"), ty("V")]);
    vec![Item::Type(declaration(
        "Map",
        TypeKind::Class,
        vec![
            bounded_param("K", vec![applied("Hashed", vec![ty("K")])]),
            type_param("V"),
        ],
        vec![applied("Iterable", vec![entry()])],
        methods(vec![
            constructor(vec![vararg("entries", entry())]),
            method(
                "put",
                vec![param("key", ty("K")), param("value", ty("V"))],
                map(),
            ),
            method("iterator", vec![], applied("Iterator", vec![entry()])),
            generic_method(
                "map",
                vec![type_param("X")],
                vec![param("f", callback(vec![ty("K"), ty("V")], ty("X")))],
                vector(ty("X")),
            ),
            generic_method(
                "mapValues",
                vec![type_param("X")],
                vec![param("f", callback(vec![ty("V")], ty("X")))],
                applied("Map", vec![ty("K"), ty("X")]),
            ),
            generic_method(
                "flatMap",
                vec![type_param("X")],
                vec![param(
                    "f",
                    callback(vec![ty("K"), ty("V")], applied("Iterable", vec![ty("X")])),
                )],
                vector(ty("X")),
            ),
            method(
                "filter",
                vec![param("f", callback(vec![ty("K"), ty("V")], ty("Bool")))],
                map(),
            ),
            generic_method(
                "fold",
                vec![type_param("X")],
                vec![
                    param("initial", ty("X")),
                    param("f", callback(vec![ty("X"), ty("K"), ty("V")], ty("X"))),
                ],
                ty("X"),
            ),
            method(
                "reduce",
                vec![param(
                    "f",
                    callback(vec![ty("K"), ty("V"), ty("K"), ty("V")], entry()),
                )],
                option(entry()),
            ),
            method(
                "exists",
                vec![param("f", callback(vec![ty("K"), ty("V")], ty("Bool")))],
                ty("Bool"),
            ),
            method(
                "forAll",
                vec![param("f", callback(vec![ty("K"), ty("V")], ty("Bool")))],
                ty("Bool"),
            ),
            method(
                "forEach",
                vec![param("f", callback(vec![ty("K"), ty("V")], ty("Unit")))],
                ty("Unit"),
            ),
            method("get", vec![param("key", ty("K"))], option(ty("V"))),
            method("contains", vec![param("key", ty("K"))], ty("Bool")),
            getter("size", ty("Int")),
            getter("isEmpty", ty("Bool")),
            getter("nonEmpty", ty("Bool")),
            method("clear", vec![], ty("Unit")),
            method("entries", vec![], vector(entry())),
            method("keys", vec![], vector(ty("K"))),
            method("values", vec![], vector(ty("V"))),
        ]),
    ))]
}
