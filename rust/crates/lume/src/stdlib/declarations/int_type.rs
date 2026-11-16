use super::super::schema::*;
use crate::ast::{Item, TypeKind};

pub(super) fn items() -> Vec<Item> {
    let mut members = vec![
        method("compareTo", vec![param("other", ty("Int"))], ty("Int")),
        method("abs", vec![], ty("Int")),
        method("pow", vec![param("exponent", ty("Int"))], ty("Int")),
        method("wrappingNeg", vec![], ty("Int")),
        method(
            "checkedPow",
            vec![param("exponent", ty("Int"))],
            option(ty("Int")),
        ),
        method("sqrt", vec![], ty("Float")),
        method("toFloat", vec![], ty("Float")),
        method("toDecimal", vec![], ty("Decimal")),
    ];
    for name in ["wrappingAdd", "wrappingSub", "wrappingMul"] {
        members.push(method(name, vec![param("other", ty("Int"))], ty("Int")));
    }
    for name in ["checkedAdd", "checkedSub", "checkedMul"] {
        members.push(method(
            name,
            vec![param("other", ty("Int"))],
            option(ty("Int")),
        ));
    }
    vec![
        Item::Type(declaration(
            "Int",
            TypeKind::Interface,
            vec![],
            vec![applied("Comparable", vec![ty("Int")])],
            methods(members),
        )),
        Item::Type(declaration(
            "Int",
            TypeKind::Object,
            vec![],
            vec![],
            methods(vec![
                getter("MIN", ty("Int")),
                getter("MAX", ty("Int")),
                method("parse", vec![param("value", ty("Str"))], option(ty("Int"))),
            ]),
        )),
    ]
}
