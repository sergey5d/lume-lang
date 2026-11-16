use super::super::schema::*;
use crate::ast::{Item, MethodDecl, TypeKind};

pub(super) fn print_methods() -> Vec<MethodDecl> {
    vec![
        method("print", vec![vararg("value", ty("Any"))], ty("Unit")),
        method("println", vec![vararg("value", ty("Any"))], ty("Unit")),
        method(
            "printf",
            vec![param("format", ty("Str")), vararg("value", ty("Any"))],
            ty("Unit"),
        ),
    ]
}

pub(super) fn items() -> Vec<Item> {
    vec![Item::Type(declaration(
        "Printer",
        TypeKind::Interface,
        vec![],
        vec![],
        methods(print_methods()),
    ))]
}
