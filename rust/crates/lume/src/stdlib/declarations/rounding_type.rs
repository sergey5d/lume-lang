use super::super::schema::*;
use crate::ast::{Item, TypeKind};

pub(super) fn items() -> Vec<Item> {
    vec![Item::Type(declaration(
        "Rounding",
        TypeKind::Enum,
        vec![],
        vec![],
        [
            "Up",
            "Down",
            "Ceiling",
            "Floor",
            "HalfUp",
            "HalfDown",
            "HalfEven",
            "Unnecessary",
        ]
        .into_iter()
        .map(|name| variant(name, vec![]))
        .collect(),
    ))]
}
