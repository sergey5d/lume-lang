use super::super::schema::*;
use crate::ast::{Item, TypeKind};

pub(super) fn items() -> Vec<Item> {
    vec![
        Item::Type(declaration(
            "Eq",
            TypeKind::Interface,
            vec![type_param("T")],
            vec![],
            methods(vec![method(
                "equals",
                vec![param("other", ty("T"))],
                ty("Bool"),
            )]),
        )),
        Item::Type(declaration(
            "Hashed",
            TypeKind::Interface,
            vec![type_param("T")],
            vec![applied("Eq", vec![ty("T")])],
            methods(vec![method("hash", vec![], ty("Int"))]),
        )),
    ]
}
