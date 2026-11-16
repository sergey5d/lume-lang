use super::super::schema::*;
use crate::ast::{Item, TypeKind, TypeMember, Visibility};

pub(super) fn items() -> Vec<Item> {
    let mut new = constructor(vec![]);
    new.visibility = Visibility::Private;
    vec![
        Item::Type(declaration(
            "RegexError",
            TypeKind::Record,
            vec![],
            vec![],
            vec![
                TypeMember::Field(field("message", ty("Str"))),
                TypeMember::Field(field("offset", option(ty("Int")))),
            ],
        )),
        Item::Type(declaration(
            "RegexMatch",
            TypeKind::Class,
            vec![],
            vec![],
            methods(vec![
                new.clone(),
                getter("text", ty("Str")),
                getter("start", ty("Int")),
                getter("end", ty("Int")),
                getter("groups", vector(option(ty("Str")))),
                getter("groupMap", applied("Map", vec![ty("Str"), ty("Str")])),
                method("group", vec![param("index", ty("Int"))], option(ty("Str"))),
                method("group", vec![param("name", ty("Str"))], option(ty("Str"))),
            ]),
        )),
        Item::Type(declaration(
            "Regex",
            TypeKind::Class,
            vec![],
            vec![],
            methods(vec![
                new,
                method("contains", vec![param("text", ty("Str"))], ty("Bool")),
                method("matches", vec![param("text", ty("Str"))], ty("Bool")),
                method(
                    "find",
                    vec![param("text", ty("Str"))],
                    option(ty("RegexMatch")),
                ),
                method(
                    "fullMatch",
                    vec![param("text", ty("Str"))],
                    option(ty("RegexMatch")),
                ),
                method(
                    "findAll",
                    vec![param("text", ty("Str"))],
                    vector(ty("RegexMatch")),
                ),
            ]),
        )),
        Item::Type(declaration(
            "Regex",
            TypeKind::Object,
            vec![],
            vec![],
            methods(vec![
                method(
                    "compile",
                    vec![param("pattern", ty("Str"))],
                    result(ty("Regex"), ty("RegexError")),
                ),
                method("escape", vec![param("text", ty("Str"))], ty("Str")),
            ]),
        )),
    ]
}
