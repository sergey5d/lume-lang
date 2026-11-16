use super::super::schema::*;
use crate::ast::{Item, TypeKind};

pub(super) fn items() -> Vec<Item> {
    let mut members = vec![
        getter("size", ty("Int")),
        getter("isEmpty", ty("Bool")),
        getter("nonEmpty", ty("Bool")),
        getter("isBlank", ty("Bool")),
        method("compareTo", vec![param("other", ty("Str"))], ty("Int")),
        method(
            "charAt",
            vec![param("index", ty("Int"))],
            option(ty("Char")),
        ),
        method("slice", vec![], ty("Str")),
        method("slice", vec![param("start", ty("Int"))], ty("Str")),
        method(
            "slice",
            vec![param("start", ty("Int")), param("end", ty("Int"))],
            ty("Str"),
        ),
        method(
            "split",
            vec![param("separator", ty("Str"))],
            vector(ty("Str")),
        ),
        method(
            "splitRegex",
            vec![param("pattern", ty("Str"))],
            vector(ty("Str")),
        ),
        method("contains", vec![param("part", ty("Str"))], ty("Bool")),
        method("startsWith", vec![param("prefix", ty("Str"))], ty("Bool")),
        method("endsWith", vec![param("suffix", ty("Str"))], ty("Bool")),
        method("indexOf", vec![param("part", ty("Str"))], option(ty("Int"))),
        method(
            "lastIndexOf",
            vec![param("part", ty("Str"))],
            option(ty("Int")),
        ),
        method("lines", vec![], vector(ty("Str"))),
        method("repeat", vec![param("count", ty("Int"))], ty("Str")),
    ];
    for name in ["trim", "trimLeft", "trimRight", "toLower", "toUpper"] {
        members.push(method(name, vec![], ty("Str")));
    }
    for name in ["replaceFirst", "replaceAll"] {
        members.push(method(
            name,
            vec![param("target", ty("Str")), param("replacement", ty("Str"))],
            ty("Str"),
        ));
    }
    for name in ["replaceFirstRegex", "replaceAllRegex"] {
        members.push(method(
            name,
            vec![param("pattern", ty("Str")), param("replacement", ty("Str"))],
            ty("Str"),
        ));
    }
    vec![Item::Type(declaration(
        "Str",
        TypeKind::Interface,
        vec![],
        vec![applied("Comparable", vec![ty("Str")])],
        methods(members),
    ))]
}
