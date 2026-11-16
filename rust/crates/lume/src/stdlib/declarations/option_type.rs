use super::super::schema::*;
use crate::ast::{ExtensionBlock, Item, TypeKind};

pub(super) fn items() -> Vec<Item> {
    vec![
        Item::Type(declaration(
            "Option",
            TypeKind::Enum,
            vec![type_param("T")],
            vec![applied("Iterable", vec![ty("T")])],
            vec![
                variant("Some", vec![field("value", ty("T"))]),
                variant("None", vec![]),
            ],
        )),
        Item::Extension(ExtensionBlock {
            target: option(ty("T")),
            methods: vec![
                getter("isSet", ty("Bool")),
                getter("isEmpty", ty("Bool")),
                method(
                    "orElse",
                    vec![param("fallback", callback(vec![], option(ty("T"))))],
                    option(ty("T")),
                ),
                generic_method(
                    "toResult",
                    vec![type_param("E")],
                    vec![param("error", callback(vec![], ty("E")))],
                    result(ty("T"), ty("E")),
                ),
                generic_method(
                    "toEither",
                    vec![type_param("L")],
                    vec![param("left", callback(vec![], ty("L")))],
                    applied("Either", vec![ty("L"), ty("T")]),
                ),
                generic_method(
                    "map",
                    vec![type_param("X")],
                    vec![param("f", callback(vec![ty("T")], ty("X")))],
                    option(ty("X")),
                ),
                generic_method(
                    "flatMap",
                    vec![type_param("X")],
                    vec![param("f", callback(vec![ty("T")], option(ty("X"))))],
                    option(ty("X")),
                ),
                method("iterator", vec![], applied("Iterator", vec![ty("T")])),
            ],
            span: span(),
        }),
        Item::Type(declaration(
            "Option",
            TypeKind::Object,
            vec![],
            vec![],
            methods(vec![generic_method(
                "when",
                vec![type_param("T")],
                vec![param("condition", ty("Bool")), param("value", ty("T"))],
                option(ty("T")),
            )]),
        )),
    ]
}
