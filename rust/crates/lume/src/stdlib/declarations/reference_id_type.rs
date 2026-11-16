use super::super::schema::*;
use crate::ast::{Item, TypeKind, Visibility};

pub(super) fn items() -> Vec<Item> {
    let mut new = constructor(vec![]);
    new.visibility = Visibility::Private;
    vec![Item::Type(declaration(
        "ReferenceId",
        TypeKind::Class,
        vec![],
        vec![applied("Hashed", vec![ty("ReferenceId")])],
        methods(vec![
            new,
            method(
                "equals",
                vec![param("other", ty("ReferenceId"))],
                ty("Bool"),
            ),
            method("hash", vec![], ty("Int")),
        ]),
    ))]
}
