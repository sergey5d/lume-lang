use super::super::schema::*;
use crate::ast::{Item, TypeKind};

pub(super) fn items() -> Vec<Item> {
    let mut members = vec![
        method("compareTo", vec![param("other", ty("Float"))], ty("Int")),
        method("pow", vec![param("exponent", ty("Int"))], ty("Float")),
        method("pow", vec![param("exponent", ty("Float"))], ty("Float")),
        method("toInt", vec![], ty("Int")),
        method("toDecimal", vec![], option(ty("Decimal"))),
    ];
    for name in ["abs", "floor", "ceil", "round", "sqrt", "log"] {
        members.push(method(name, vec![], ty("Float")));
    }
    vec![
        Item::Type(declaration(
            "Float",
            TypeKind::Interface,
            vec![],
            vec![applied("Comparable", vec![ty("Float")])],
            methods(members),
        )),
        Item::Type(declaration(
            "Float",
            TypeKind::Object,
            vec![],
            vec![],
            methods(vec![
                getter("MIN", ty("Float")),
                getter("MAX", ty("Float")),
                method(
                    "parse",
                    vec![param("value", ty("Str"))],
                    option(ty("Float")),
                ),
            ]),
        )),
    ]
}
