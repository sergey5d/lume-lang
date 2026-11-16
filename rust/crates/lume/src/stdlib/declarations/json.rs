use super::super::schema::*;
use crate::ast::{ExtensionBlock, Item, TypeKind, TypeMember};

pub(super) fn items() -> Vec<Item> {
    vec![
        Item::Type(declaration(
            "JsonName",
            TypeKind::Annotation,
            vec![],
            vec![],
            vec![TypeMember::Field(field("value", ty("Str")))],
        )),
        Item::Type(declaration(
            "JsonIgnore",
            TypeKind::Annotation,
            vec![],
            vec![],
            vec![],
        )),
        Item::Type(declaration(
            "JsonField",
            TypeKind::Record,
            vec![],
            vec![],
            vec![
                TypeMember::Field(field("name", ty("Str"))),
                TypeMember::Field(field("value", ty("JsonValue"))),
            ],
        )),
        Item::Type(declaration(
            "JsonValue",
            TypeKind::Enum,
            vec![],
            vec![],
            vec![
                variant("JsonNull", vec![]),
                variant("JsonBool", vec![field("value", ty("Bool"))]),
                variant("JsonNumber", vec![field("value", ty("Str"))]),
                variant("JsonString", vec![field("value", ty("Str"))]),
                variant("JsonArray", vec![field("values", vector(ty("JsonValue")))]),
                variant("JsonObject", vec![field("fields", vector(ty("JsonField")))]),
            ],
        )),
        Item::Extension(ExtensionBlock {
            target: ty("JsonValue"),
            methods: vec![
                method("stringify", vec![], ty("Str")),
                method("textContent", vec![], ty("Str")),
            ],
            span: span(),
        }),
        Item::Type(declaration(
            "Json",
            TypeKind::Object,
            vec![],
            vec![],
            methods(vec![
                method("str", vec![param("value", ty("Str"))], ty("JsonValue")),
                method("int", vec![param("value", ty("Int"))], ty("JsonValue")),
                method("float", vec![param("value", ty("Float"))], ty("JsonValue")),
                method("bool", vec![param("value", ty("Bool"))], ty("JsonValue")),
                method("nil", vec![], ty("JsonValue")),
                method(
                    "field",
                    vec![param("name", ty("Str")), param("value", ty("JsonValue"))],
                    ty("JsonField"),
                ),
                method(
                    "array",
                    vec![vararg("values", ty("JsonValue"))],
                    ty("JsonValue"),
                ),
                method(
                    "obj",
                    vec![vararg("fields", ty("JsonField"))],
                    ty("JsonValue"),
                ),
                method("obj", vec![param("value", ty("Any"))], ty("JsonValue")),
                method("encode", vec![param("value", ty("Any"))], ty("JsonValue")),
                method("stringify", vec![param("value", ty("Any"))], ty("Str")),
                method(
                    "textContent",
                    vec![param("value", ty("JsonValue"))],
                    ty("Str"),
                ),
                generic_method(
                    "decode",
                    vec![reified_param("T")],
                    vec![param("text", ty("Str"))],
                    result(ty("T"), ty("Str")),
                ),
            ]),
        )),
    ]
}
