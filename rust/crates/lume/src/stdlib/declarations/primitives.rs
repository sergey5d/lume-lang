use super::super::schema::*;
use crate::ast::{Item, TypeKind};

pub(super) fn items() -> Vec<Item> {
    let mut items = ["Bool", "Unit", "Never"]
        .into_iter()
        .map(|name| {
            Item::Type(declaration(
                name,
                TypeKind::Interface,
                vec![],
                vec![],
                vec![],
            ))
        })
        .collect::<Vec<_>>();
    items.push(Item::Type(declaration(
        "Any",
        TypeKind::Interface,
        vec![],
        vec![],
        methods(vec![
            method("toStr", vec![], ty("Str")),
            method("equals", vec![param("other", ty("Any"))], ty("Bool")),
        ]),
    )));
    items
}
