use super::super::schema::*;
use crate::ast::{Item, TypeKind, TypeMember};

pub(super) fn items() -> Vec<Item> {
    let mut members = ["start", "end", "step"]
        .into_iter()
        .map(|name| TypeMember::Field(field(name, ty("Int"))))
        .collect::<Vec<_>>();
    members.extend(methods(vec![
        method("iterator", vec![], applied("Iterator", vec![ty("Int")])),
        generic_method(
            "zip",
            vec![type_param("X")],
            vec![param("other", vector(ty("X")))],
            vector(tuple(vec![ty("Int"), ty("X")])),
        ),
        method(
            "zipWithIndex",
            vec![],
            vector(tuple(vec![ty("Int"), ty("Int")])),
        ),
    ]));
    vec![
        function(
            "Range",
            vec![],
            vec![
                param("start", ty("Int")),
                param("end", ty("Int")),
                default_param("step", ty("Int"), integer(1)),
            ],
            ty("IntRange"),
        ),
        Item::Type(declaration(
            "IntRange",
            TypeKind::Class,
            vec![],
            vec![applied("Iterable", vec![ty("Int")])],
            members,
        )),
    ]
}
