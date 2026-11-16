use super::super::schema::*;
use crate::ast::{Item, TypeKind};

pub(super) fn items() -> Vec<Item> {
    let mut members = Vec::new();
    for name in ["min", "max"] {
        for numeric in ["Int", "Float"] {
            members.push(method(
                name,
                vec![param("left", ty(numeric)), param("right", ty(numeric))],
                ty(numeric),
            ));
        }
    }
    vec![Item::Type(declaration(
        "Math",
        TypeKind::Object,
        vec![],
        vec![],
        methods(members),
    ))]
}
