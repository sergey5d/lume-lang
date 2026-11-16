use super::super::schema::*;
use crate::ast::{ExtensionBlock, Item, TypeKind};

pub(super) fn items() -> Vec<Item> {
    vec![
        Item::Type(declaration(
            "Result",
            TypeKind::Enum,
            vec![type_param("T"), type_param("E")],
            vec![],
            vec![
                variant("Ok", vec![field("value", ty("T"))]),
                variant("Err", vec![field("error", ty("E"))]),
            ],
        )),
        Item::Extension(ExtensionBlock {
            target: result(ty("T"), ty("E")),
            methods: vec![
                getter("isOk", ty("Bool")),
                getter("isErr", ty("Bool")),
                method("expectErr", vec![], ty("E")),
                method(
                    "orElse",
                    vec![param(
                        "fallback",
                        callback(vec![], result(ty("T"), ty("E"))),
                    )],
                    result(ty("T"), ty("E")),
                ),
                method("toOption", vec![], option(ty("T"))),
                method(
                    "toEither",
                    vec![],
                    applied("Either", vec![ty("E"), ty("T")]),
                ),
                generic_method(
                    "map",
                    vec![type_param("X")],
                    vec![param("f", callback(vec![ty("T")], ty("X")))],
                    result(ty("X"), ty("E")),
                ),
                generic_method(
                    "mapError",
                    vec![type_param("X")],
                    vec![param("f", callback(vec![ty("E")], ty("X")))],
                    result(ty("T"), ty("X")),
                ),
                generic_method(
                    "flatMap",
                    vec![type_param("X")],
                    vec![param(
                        "f",
                        callback(vec![ty("T")], result(ty("X"), ty("E"))),
                    )],
                    result(ty("X"), ty("E")),
                ),
            ],
            span: span(),
        }),
    ]
}
