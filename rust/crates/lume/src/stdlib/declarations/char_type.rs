use super::super::schema::*;
use crate::ast::{Item, TypeKind};

pub(super) fn items() -> Vec<Item> {
    vec![Item::Type(declaration(
        "Char",
        TypeKind::Interface,
        vec![],
        vec![],
        methods(vec![method(
            "repeat",
            vec![param("count", ty("Int"))],
            ty("Str"),
        )]),
    ))]
}
