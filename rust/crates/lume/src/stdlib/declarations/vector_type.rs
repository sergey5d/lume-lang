use super::super::schema::*;
use crate::ast::{Item, TypeKind};

pub(super) fn items() -> Vec<Item> {
    let mut flatten = generic_method("flatten", vec![type_param("X")], vec![], vector(ty("X")));
    flatten
        .type_conditions
        .push(bound(ty("T"), applied("Iterable", vec![ty("X")])));
    let mut sort = method("sort", vec![], vector(ty("T")));
    sort.type_conditions
        .push(bound(ty("T"), applied("Comparable", vec![ty("T")])));

    vec![
        Item::Type(declaration(
            "Vector",
            TypeKind::Class,
            vec![type_param("T")],
            vec![applied("Iterable", vec![ty("T")])],
            methods(vec![
                constructor(vec![vararg("values", ty("T"))]),
                method("add", vec![param("value", ty("T"))], vector(ty("T"))),
                method(
                    "addAll",
                    vec![param("values", applied("Iterable", vec![ty("T")]))],
                    vector(ty("T")),
                ),
                generic_method(
                    "map",
                    vec![type_param("X")],
                    vec![param("f", callback(vec![ty("T")], ty("X")))],
                    vector(ty("X")),
                ),
                generic_method(
                    "flatMap",
                    vec![type_param("X")],
                    vec![param(
                        "f",
                        callback(vec![ty("T")], applied("Iterable", vec![ty("X")])),
                    )],
                    vector(ty("X")),
                ),
                flatten,
                method(
                    "filter",
                    vec![param("f", callback(vec![ty("T")], ty("Bool")))],
                    vector(ty("T")),
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
                method("contains", vec![param("value", ty("T"))], ty("Bool")),
                method("indexOf", vec![param("value", ty("T"))], option(ty("Int"))),
                sort,
                method(
                    "sort",
                    vec![param(
                        "compare",
                        callback(vec![ty("T"), ty("T")], ty("Int")),
                    )],
                    vector(ty("T")),
                ),
                method("take", vec![param("count", ty("Int"))], vector(ty("T"))),
                generic_method(
                    "zip",
                    vec![type_param("X")],
                    vec![param("other", vector(ty("X")))],
                    vector(tuple(vec![ty("T"), ty("X")])),
                ),
                method(
                    "zipWithIndex",
                    vec![],
                    vector(tuple(vec![ty("T"), ty("Int")])),
                ),
                method("at", vec![param("index", ty("Int"))], option(ty("T"))),
                method("slice", vec![], vector(ty("T"))),
                method("slice", vec![param("start", ty("Int"))], vector(ty("T"))),
                method(
                    "slice",
                    vec![param("start", ty("Int")), param("end", ty("Int"))],
                    vector(ty("T")),
                ),
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
                method("tail", vec![], vector(ty("T"))),
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
        )),
        Item::Type(declaration(
            "Vector",
            TypeKind::Object,
            vec![],
            vec![],
            methods(vec![
                generic_method(
                    "fill",
                    vec![type_param("T")],
                    vec![param("length", ty("Int")), param("value", ty("T"))],
                    vector(ty("T")),
                ),
                generic_method(
                    "generate",
                    vec![type_param("T")],
                    vec![
                        param("length", ty("Int")),
                        param("generator", callback(vec![ty("Int")], ty("T"))),
                    ],
                    vector(ty("T")),
                ),
                generic_method(
                    "from",
                    vec![type_param("T")],
                    vec![param("values", applied("Iterable", vec![ty("T")]))],
                    vector(ty("T")),
                ),
            ]),
        )),
    ]
}
