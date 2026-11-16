use super::super::schema::*;
use crate::ast::{Item, TypeKind, TypeMember};

pub(super) fn items() -> Vec<Item> {
    vec![Item::Type(declaration(
        "InvalidIndex",
        TypeKind::Record,
        vec![],
        vec![],
        vec![
            TypeMember::Field(field("index", ty("Int"))),
            TypeMember::Field(field("size", ty("Int"))),
        ],
    ))]
}
