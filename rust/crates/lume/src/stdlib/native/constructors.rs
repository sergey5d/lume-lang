use super::*;

const CONSTRUCTORS: &[(&str, BuiltinMethodFn)] = &[
    ("Range", |i, r, a, s| i.native_ctor_range(r, a, s)),
    ("IntRange", |i, r, a, s| i.native_ctor_int_range(r, a, s)),
    ("Vector", |i, r, a, s| i.native_ctor_list(r, a, s)),
    ("Array", |i, r, a, s| i.native_ctor_list(r, a, s)),
    ("LinkedList", |i, r, a, s| i.native_ctor_list(r, a, s)),
    ("Set", |i, r, a, s| i.native_ctor_set(r, a, s)),
    ("Map", |i, r, a, s| i.native_ctor_map(r, a, s)),
    ("Some", |i, r, a, s| i.native_ctor_some(r, a, s)),
    ("None", |i, r, a, s| i.native_ctor_none(r, a, s)),
    ("Ok", |i, r, a, s| i.native_ctor_ok(r, a, s)),
    ("Err", |i, r, a, s| i.native_ctor_err(r, a, s)),
    ("Left", |i, r, a, s| i.native_ctor_left(r, a, s)),
    ("Right", |i, r, a, s| i.native_ctor_right(r, a, s)),
];

const ROOT_FUNCTIONS: &[(&str, BuiltinMethodFn)] = &[
    ("Range", |i, r, a, s| i.native_ctor_range(r, a, s)),
    ("print", |i, r, a, s| i.native_os_print(r, a, s)),
    ("println", |i, r, a, s| i.native_os_println(r, a, s)),
    ("printf", |i, r, a, s| i.native_os_printf(r, a, s)),
    ("panic", |i, r, a, s| i.native_predef_panic(r, a, s)),
    ("assert", |i, r, a, s| i.native_predef_assert(r, a, s)),
    ("ensure", |i, r, a, s| i.native_predef_ensure(r, a, s)),
    ("identity", |i, r, a, s| i.native_predef_identity(r, a, s)),
];

pub(crate) fn constructor(name: &str) -> Option<BuiltinMethodFn> {
    CONSTRUCTORS
        .iter()
        .find(|(candidate, _)| *candidate == name)
        .map(|(_, handler)| *handler)
}

pub(crate) fn root_function(name: &str) -> Option<BuiltinMethodFn> {
    ROOT_FUNCTIONS
        .iter()
        .find(|(candidate, _)| *candidate == name)
        .map(|(_, handler)| *handler)
}

pub(crate) fn constructor_owner(name: &str) -> Option<(&'static str, TypeKind)> {
    const OWNERS: &[(&str, &str, TypeKind)] = &[
        ("Range", "IntRange", TypeKind::Class),
        ("IntRange", "IntRange", TypeKind::Class),
        ("Vector", "Vector", TypeKind::Class),
        ("Array", "Array", TypeKind::Class),
        ("LinkedList", "LinkedList", TypeKind::Class),
        ("Set", "Set", TypeKind::Class),
        ("Map", "Map", TypeKind::Class),
        ("Some", "Option", TypeKind::Enum),
        ("None", "Option", TypeKind::Enum),
        ("Ok", "Result", TypeKind::Enum),
        ("Err", "Result", TypeKind::Enum),
        ("Left", "Either", TypeKind::Enum),
        ("Right", "Either", TypeKind::Enum),
    ];
    OWNERS
        .iter()
        .find(|(candidate, _, _)| *candidate == name)
        .map(|(_, owner, kind)| (*owner, *kind))
}

pub(crate) fn root_function_for_ir(function: &ir::Function) -> Option<BuiltinMethodFn> {
    if function.kind != ir::FunctionKind::TopLevel {
        return None;
    }
    let canonical = super::super::declarations().items.iter().any(|item| {
        matches!(item, ast::Item::Function(declaration) if declaration.name == function.name && function.span == Some(declaration.span))
    });
    canonical.then(|| root_function(&function.name)).flatten()
}
