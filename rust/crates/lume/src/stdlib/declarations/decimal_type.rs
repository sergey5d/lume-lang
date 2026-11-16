use super::super::schema::*;
use crate::ast::{Item, TypeKind};

pub(super) fn items() -> Vec<Item> {
    vec![
        Item::Type(declaration(
            "Decimal",
            TypeKind::Interface,
            vec![],
            vec![
                applied("Hashed", vec![ty("Decimal")]),
                applied("Comparable", vec![ty("Decimal")]),
            ],
            methods(vec![
                getter("scale", ty("Int")),
                method("compareTo", vec![param("other", ty("Decimal"))], ty("Int")),
                method("equals", vec![param("other", ty("Decimal"))], ty("Bool")),
                method("hash", vec![], ty("Int")),
                method("abs", vec![], ty("Decimal")),
                method(
                    "divide",
                    vec![
                        param("other", ty("Decimal")),
                        param("scale", ty("Int")),
                        param("rounding", ty("Rounding")),
                    ],
                    ty("Decimal"),
                ),
                method(
                    "round",
                    vec![param("scale", ty("Int")), param("rounding", ty("Rounding"))],
                    ty("Decimal"),
                ),
                method("toInt", vec![], option(ty("Int"))),
                method("toFloat", vec![], ty("Float")),
            ]),
        )),
        Item::Type(declaration(
            "Decimal",
            TypeKind::Object,
            vec![],
            vec![],
            methods(vec![method(
                "parse",
                vec![param("value", ty("Str"))],
                option(ty("Decimal")),
            )]),
        )),
    ]
}
