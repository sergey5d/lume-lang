use super::super::schema::*;
use crate::ast::{Item, TypeKind};

pub(super) fn items() -> Vec<Item> {
    vec![Item::Type(declaration(
        "Comparable",
        TypeKind::Interface,
        vec![type_param("T")],
        vec![],
        methods(vec![method(
            "compareTo",
            vec![param("other", ty("T"))],
            ty("Int"),
        )]),
    ))]
}
