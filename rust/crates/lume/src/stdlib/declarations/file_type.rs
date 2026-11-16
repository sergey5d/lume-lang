use super::super::schema::*;
use crate::ast::{Item, MethodDecl, TypeKind, Visibility};

fn byte_reader_methods() -> Vec<MethodDecl> {
    vec![
        method(
            "read",
            vec![param("maxBytes", ty("Int"))],
            result(vector(ty("Byte")), ty("FileError")),
        ),
        method(
            "readToEnd",
            vec![],
            result(vector(ty("Byte")), ty("FileError")),
        ),
    ]
}

fn seekable_methods() -> Vec<MethodDecl> {
    vec![
        getter("position", ty("Int")),
        method(
            "seek",
            vec![param("offset", ty("Int"))],
            result(ty("Int"), ty("FileError")),
        ),
        method(
            "seek",
            vec![param("offset", ty("Int")), param("from", ty("SeekFrom"))],
            result(ty("Int"), ty("FileError")),
        ),
    ]
}

fn closeable_methods() -> Vec<MethodDecl> {
    vec![
        getter("closed", ty("Bool")),
        method("close", vec![], result(ty("Unit"), ty("FileError"))),
    ]
}

fn text_reader_methods() -> Vec<MethodDecl> {
    vec![
        method(
            "readLine",
            vec![],
            result(option(ty("Str")), ty("FileError")),
        ),
        method("readToEnd", vec![], result(ty("Str"), ty("FileError"))),
    ]
}

pub(super) fn items() -> Vec<Item> {
    let mut new = constructor(vec![]);
    new.visibility = Visibility::Private;
    let mut stream = vec![new.clone(), getter("path", ty("Str"))];
    stream.extend(byte_reader_methods());
    stream.extend(seekable_methods());
    stream.extend(closeable_methods());
    let mut text = vec![new, getter("path", ty("Str"))];
    text.extend(closeable_methods());
    text.extend(text_reader_methods());

    vec![
        alias("Byte", ty("Int")),
        Item::Type(declaration(
            "FileError",
            TypeKind::Enum,
            vec![],
            vec![],
            vec![
                variant("NotFound", vec![field("path", ty("Str"))]),
                variant("AccessDenied", vec![field("path", ty("Str"))]),
                variant("Closed", vec![field("path", ty("Str"))]),
                variant(
                    "InvalidEncoding",
                    vec![field("path", ty("Str")), field("offset", ty("Int"))],
                ),
                variant(
                    "IoFailure",
                    vec![
                        field("operation", ty("Str")),
                        field("path", ty("Str")),
                        field("message", ty("Str")),
                    ],
                ),
            ],
        )),
        Item::Type(declaration(
            "SeekFrom",
            TypeKind::Enum,
            vec![],
            vec![],
            vec![
                variant("Start", vec![]),
                variant("Current", vec![]),
                variant("End", vec![]),
            ],
        )),
        Item::Type(declaration(
            "Closeable",
            TypeKind::Interface,
            vec![],
            vec![],
            methods(closeable_methods()),
        )),
        Item::Type(declaration(
            "ByteReader",
            TypeKind::Interface,
            vec![],
            vec![],
            methods(byte_reader_methods()),
        )),
        Item::Type(declaration(
            "Seekable",
            TypeKind::Interface,
            vec![],
            vec![],
            methods(seekable_methods()),
        )),
        Item::Type(declaration(
            "TextReader",
            TypeKind::Interface,
            vec![],
            vec![ty("Closeable")],
            methods(text_reader_methods()),
        )),
        Item::Type(declaration(
            "FileStream",
            TypeKind::Class,
            vec![],
            vec![ty("ByteReader"), ty("Seekable"), ty("Closeable")],
            methods(stream),
        )),
        Item::Type(declaration(
            "TextFileReader",
            TypeKind::Class,
            vec![],
            vec![ty("TextReader")],
            methods(text),
        )),
        Item::Type(declaration(
            "File",
            TypeKind::Object,
            vec![],
            vec![],
            methods(vec![
                method(
                    "readBytes",
                    vec![param("path", ty("Str"))],
                    result(vector(ty("Byte")), ty("FileError")),
                ),
                method(
                    "readText",
                    vec![param("path", ty("Str"))],
                    result(ty("Str"), ty("FileError")),
                ),
                method(
                    "open",
                    vec![param("path", ty("Str"))],
                    result(ty("FileStream"), ty("FileError")),
                ),
                method(
                    "openText",
                    vec![param("path", ty("Str"))],
                    result(ty("TextFileReader"), ty("FileError")),
                ),
            ]),
        )),
    ]
}
