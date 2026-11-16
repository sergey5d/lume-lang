use super::super::schema::*;
use crate::ast::{Item, TypeKind, TypeMember};

pub(super) fn items() -> Vec<Item> {
    let mut members = vec![
        TypeMember::Field(field("stdout", ty("Printer"))),
        TypeMember::Field(field("stderr", ty("Printer"))),
    ];
    members.extend(methods(vec![
        getter("args", vector(ty("Str"))),
        getter("env", applied("Map", vec![ty("Str"), ty("Str")])),
    ]));
    members.extend(methods(super::printer_type::print_methods()));
    vec![Item::Type(declaration(
        "OS",
        TypeKind::Object,
        vec![],
        vec![ty("Printer")],
        members,
    ))]
}
