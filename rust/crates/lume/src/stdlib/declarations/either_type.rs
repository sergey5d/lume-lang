use super::super::schema::*;
use crate::ast::{ExtensionBlock, Item, TypeKind};

pub(super) fn items() -> Vec<Item> {
    let either = || applied("Either", vec![ty("L"), ty("R")]);
    let mut merge = method("merge", vec![], ty("L"));
    merge.type_conditions.push(equal(ty("L"), ty("R")));
    vec![
        Item::Type(declaration(
            "Either",
            TypeKind::Enum,
            vec![type_param("L"), type_param("R")],
            vec![],
            vec![
                variant("Left", vec![field("value", ty("L"))]),
                variant("Right", vec![field("value", ty("R"))]),
            ],
        )),
        Item::Extension(ExtensionBlock {
            target: either(),
            methods: vec![
                getter("isLeft", ty("Bool")),
                getter("isRight", ty("Bool")),
                method("expectRight", vec![], ty("R")),
                method("expectLeft", vec![], ty("L")),
                method(
                    "orElse",
                    vec![param("fallback", callback(vec![], either()))],
                    either(),
                ),
                generic_method(
                    "map",
                    vec![type_param("X")],
                    vec![param("f", callback(vec![ty("R")], ty("X")))],
                    applied("Either", vec![ty("L"), ty("X")]),
                ),
                generic_method(
                    "mapLeft",
                    vec![type_param("X")],
                    vec![param("f", callback(vec![ty("L")], ty("X")))],
                    applied("Either", vec![ty("X"), ty("R")]),
                ),
                generic_method(
                    "flatMap",
                    vec![type_param("X")],
                    vec![param(
                        "f",
                        callback(vec![ty("R")], applied("Either", vec![ty("L"), ty("X")])),
                    )],
                    applied("Either", vec![ty("L"), ty("X")]),
                ),
                merge,
                method("toOption", vec![], option(ty("R"))),
                method("toResult", vec![], result(ty("R"), ty("L"))),
            ],
            span: span(),
        }),
    ]
}
