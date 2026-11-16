use super::super::schema::*;
use crate::ast::{Item, TypeKind};

pub(super) fn items() -> Vec<Item> {
    vec![
        Item::Type(declaration(
            "Iterator",
            TypeKind::Interface,
            vec![type_param("T")],
            vec![],
            methods(vec![
                getter("hasNext", ty("Bool")),
                method("next", vec![], ty("T")),
            ]),
        )),
        Item::Type(declaration(
            "Iterable",
            TypeKind::Interface,
            vec![type_param("T")],
            vec![],
            methods(vec![method(
                "iterator",
                vec![],
                applied("Iterator", vec![ty("T")]),
            )]),
        )),
    ]
}
