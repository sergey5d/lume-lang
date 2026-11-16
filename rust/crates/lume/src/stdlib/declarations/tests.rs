use std::collections::{BTreeMap, HashMap, HashSet};

use super::*;
use crate::ast::{
    Expr, ExtensionBlock, FunctionDecl, GenericCondition, Item, MethodDecl, Param, Program,
    TypeDecl, TypeKind, TypeMember, TypeParam, TypeRef, Visibility,
};

fn type_decl<'a>(program: &'a Program, name: &str, kind: TypeKind) -> &'a TypeDecl {
    program
        .items
        .iter()
        .find_map(|item| match item {
            Item::Type(declaration) if declaration.name == name && declaration.kind == kind => {
                Some(declaration)
            }
            _ => None,
        })
        .unwrap_or_else(|| panic!("missing {kind:?} {name}"))
}

fn function_decl<'a>(program: &'a Program, name: &str) -> &'a FunctionDecl {
    program
        .items
        .iter()
        .find_map(|item| match item {
            Item::Function(function) if function.name == name => Some(function),
            _ => None,
        })
        .unwrap_or_else(|| panic!("missing function {name}"))
}

fn extension<'a>(program: &'a Program, name: &str) -> &'a ExtensionBlock {
    program.items.iter().find_map(|item| match item {
        Item::Extension(extension) if matches!(&extension.target, TypeRef::Named { name: target, .. } if target == name) => Some(extension),
        _ => None,
    }).unwrap_or_else(|| panic!("missing extension {name}"))
}

fn members(declaration: &TypeDecl) -> Vec<&MethodDecl> {
    declaration
        .members
        .iter()
        .filter_map(|member| match member {
            TypeMember::Method(method) => Some(method),
            _ => None,
        })
        .collect()
}

fn arities(program: &Program) -> HashMap<String, usize> {
    program
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Type(declaration) if declaration.kind != TypeKind::Object => {
                Some((declaration.name.clone(), declaration.type_params.len()))
            }
            Item::TypeAlias(alias) => Some((alias.name.clone(), alias.type_params.len())),
            _ => None,
        })
        .collect()
}

fn check_type(reference: &TypeRef, types: &HashMap<String, usize>, variables: &HashSet<String>) {
    match reference {
        TypeRef::Named { name, args, .. } => {
            assert_ne!(name, "Unknown");
            if variables.contains(name) {
                assert!(args.is_empty(), "type variable {name} has arguments");
            } else {
                assert_eq!(types.get(name), Some(&args.len()), "invalid type {name}");
            }
            for argument in args {
                check_type(argument, types, variables);
            }
        }
        TypeRef::Function { params, ret, .. } => {
            for param in params {
                check_type(param, types, variables);
            }
            check_type(ret, types, variables);
        }
        TypeRef::Tuple { fields, .. } => {
            for field in fields {
                check_type(&field.ty, types, variables);
            }
        }
        TypeRef::Record { fields, .. } => {
            for field in fields {
                check_type(&field.ty, types, variables);
            }
        }
        TypeRef::Union { members, .. } => {
            for member in members {
                check_type(member, types, variables);
            }
        }
        TypeRef::Wildcard { .. } => {}
    }
}

fn check_conditions(
    conditions: &[GenericCondition],
    types: &HashMap<String, usize>,
    variables: &HashSet<String>,
) {
    for condition in conditions {
        let (left, right) = match condition {
            GenericCondition::Bound { subject, bound, .. } => (subject, bound),
            GenericCondition::Equal { left, right, .. } => (left, right),
        };
        check_type(left, types, variables);
        check_type(right, types, variables);
    }
}

fn check_params(params: &[Param], types: &HashMap<String, usize>, variables: &HashSet<String>) {
    for (index, param) in params.iter().enumerate() {
        check_type(
            param.ty.as_ref().expect("native parameter type"),
            types,
            variables,
        );
        if param.variadic {
            assert_eq!(index, params.len() - 1, "vararg must be last");
            assert!(
                matches!(&param.ty, Some(TypeRef::Named { name, args, .. }) if name == "Vector" && args.len() == 1)
            );
        }
    }
}

fn check_type_params(
    params: &[TypeParam],
    types: &HashMap<String, usize>,
    variables: &HashSet<String>,
) {
    for param in params {
        for bound in &param.bounds {
            check_type(bound, types, variables);
        }
    }
}

fn check_method(method: &MethodDecl, types: &HashMap<String, usize>, owner: &HashSet<String>) {
    let mut variables = owner.clone();
    variables.extend(method.type_params.iter().map(|param| param.name.clone()));
    check_type_params(&method.type_params, types, &variables);
    check_conditions(&method.type_conditions, types, &variables);
    check_params(&method.params, types, &variables);
    if method.name == "new" {
        assert!(method.return_type.is_none());
    } else {
        check_type(
            method
                .return_type
                .as_ref()
                .expect("native method return type"),
            types,
            &variables,
        );
    }
    if method.getter {
        assert!(method.params.is_empty());
    }
}

fn check_program(program: &Program, types: &HashMap<String, usize>) {
    for item in &program.items {
        match item {
            Item::Type(declaration) => {
                assert_eq!(declaration.visibility, Visibility::Default);
                let variables = declaration
                    .type_params
                    .iter()
                    .map(|param| param.name.clone())
                    .collect();
                check_type_params(&declaration.type_params, types, &variables);
                check_conditions(&declaration.type_conditions, types, &variables);
                for bound in &declaration.with_bounds {
                    check_type(bound, types, &variables);
                }
                for member in &declaration.members {
                    match member {
                        TypeMember::Method(method) => {
                            check_method(method, types, &variables);
                            assert_eq!(
                                method.body.is_none(),
                                declaration.kind == TypeKind::Interface,
                                "incorrect body marker for {}.{}",
                                declaration.name,
                                method.name
                            );
                        }
                        TypeMember::Field(field) => {
                            check_type(field.ty.as_ref().expect("field type"), types, &variables)
                        }
                        TypeMember::Case(case) => {
                            for field in &case.fields {
                                check_type(
                                    field.ty.as_ref().expect("payload type"),
                                    types,
                                    &variables,
                                );
                            }
                        }
                    }
                }
            }
            Item::Function(function) => {
                let variables = function
                    .type_params
                    .iter()
                    .map(|param| param.name.clone())
                    .collect();
                check_type_params(&function.type_params, types, &variables);
                check_conditions(&function.type_conditions, types, &variables);
                check_params(&function.params, types, &variables);
                check_type(
                    function.return_type.as_ref().expect("function return type"),
                    types,
                    &variables,
                );
            }
            Item::TypeAlias(alias) => check_type(&alias.target, types, &HashSet::new()),
            Item::Extension(extension) => {
                let TypeRef::Named { args, .. } = &extension.target else {
                    panic!("named extension target");
                };
                let variables = args
                    .iter()
                    .map(|argument| {
                        let TypeRef::Named { name, args, .. } = argument else {
                            panic!("extension type parameter");
                        };
                        assert!(args.is_empty());
                        name.clone()
                    })
                    .collect();
                check_type(&extension.target, types, &variables);
                for method in &extension.methods {
                    check_method(method, types, &variables);
                    assert!(method.body.is_some());
                }
            }
            Item::Statement(_) => {
                panic!("native contracts have no executable top-level statements")
            }
        }
    }
}

#[test]
fn all_contracts_are_fully_typed_and_have_appropriate_body_markers() {
    let program = program();
    let types = arities(&program);
    check_program(&program, &types);
    for path in ["lume/json", "lume/regex"] {
        let module = module_program(path).unwrap();
        let mut types = types.clone();
        types.extend(arities(&module));
        check_program(&module, &types);
    }
}

#[test]
fn collection_method_sets_preserve_overloads_and_inherited_iteration() {
    let program = program();
    for (name, expected) in [
        (
            "Vector",
            "new add addAll map flatMap flatten filter fold reduce exists forAll contains indexOf sort sort take zip zipWithIndex at slice slice slice setAt insertAt removeAt first last tail isEmpty nonEmpty removeFirst removeLast size forEach iterator makeStr makeStr",
        ),
        (
            "Array",
            "new at setAt first last clone map forEach count exists forAll contains find indexOf zip zipWithIndex size iterator",
        ),
        (
            "LinkedList",
            "new add addAll map flatMap filter fold reduce exists forAll indexOf zip zipWithIndex at setAt insertAt removeAt first last tail isEmpty nonEmpty removeFirst removeLast size forEach iterator makeStr makeStr",
        ),
        (
            "Set",
            "new add addAll iterator map flatMap filter fold reduce exists forAll forEach contains isEmpty nonEmpty size makeStr makeStr",
        ),
        (
            "Map",
            "new put iterator map mapValues flatMap filter fold reduce exists forAll forEach get contains size isEmpty nonEmpty clear entries keys values",
        ),
    ] {
        let mut expected = expected.split_whitespace().collect::<Vec<_>>();
        let declaration = type_decl(&program, name, TypeKind::Class);
        let mut actual = members(declaration)
            .iter()
            .map(|method| method.name.as_str())
            .collect::<Vec<_>>();
        actual.sort_unstable();
        expected.sort_unstable();
        assert_eq!(actual, expected, "{name} methods");
        assert!(
            matches!(&declaration.with_bounds[..], [TypeRef::Named { name, .. }] if name == "Iterable")
        );
    }
}

#[test]
fn generic_constraints_are_not_erased() {
    let program = program();
    let vector = type_decl(&program, "Vector", TypeKind::Class);
    let methods = members(vector);
    let flatten = methods
        .iter()
        .find(|method| method.name == "flatten")
        .unwrap();
    assert_eq!(flatten.type_params, vec![type_param("X")]);
    assert_eq!(
        flatten.type_conditions,
        vec![bound(ty("T"), applied("Iterable", vec![ty("X")]))]
    );
    let sort = methods
        .iter()
        .find(|method| method.name == "sort" && method.params.is_empty())
        .unwrap();
    assert_eq!(
        sort.type_conditions,
        vec![bound(ty("T"), applied("Comparable", vec![ty("T")]))]
    );
    let map = type_decl(&program, "Map", TypeKind::Class);
    assert_eq!(
        map.type_params[0],
        bounded_param("K", vec![applied("Hashed", vec![ty("K")])])
    );
    let merge = extension(&program, "Either")
        .methods
        .iter()
        .find(|method| method.name == "merge")
        .unwrap();
    assert_eq!(merge.type_conditions, vec![equal(ty("L"), ty("R"))]);
}

#[test]
fn defaults_are_real_typed_expressions() {
    let program = program();
    let assert = function_decl(&program, "assert");
    assert_eq!(assert.params[1].ty, Some(ty("Str")));
    assert_eq!(
        assert.params[1].initializer,
        Some(string("assert condition was false"))
    );
    let range = function_decl(&program, "Range");
    assert_eq!(range.params[2].ty, Some(ty("Int")));
    assert!(matches!(&range.params[2].initializer, Some(Expr::Integer { raw, .. }) if raw == "1"));
}

#[test]
fn factory_objects_only_expose_supported_operations() {
    let program = program();
    for (owner, names) in [
        ("Vector", vec!["fill", "generate", "from"]),
        ("Array", vec!["fill", "generate", "from"]),
        ("Set", vec!["from"]),
        ("Option", vec!["when"]),
    ] {
        let declaration = type_decl(&program, owner, TypeKind::Object);
        let actual = members(declaration);
        assert_eq!(
            actual
                .iter()
                .map(|method| method.name.as_str())
                .collect::<Vec<_>>(),
            names
        );
        assert!(
            actual
                .iter()
                .all(|method| method.type_params == vec![type_param("T")])
        );
    }
    let option = type_decl(&program, "Option", TypeKind::Object);
    assert_eq!(members(option)[0].params[1].ty, Some(ty("T")));
    let vector = type_decl(&program, "Vector", TypeKind::Object);
    let generate = members(vector)
        .into_iter()
        .find(|method| method.name == "generate")
        .unwrap();
    assert_eq!(
        generate.params[1].ty,
        Some(callback(vec![ty("Int")], ty("T")))
    );
}

#[test]
fn array_from_preserves_the_iterable_element_type() {
    let program = program();
    let array = type_decl(&program, "Array", TypeKind::Object);
    let from = members(array)
        .into_iter()
        .find(|method| method.name == "from")
        .expect("Array.from factory declaration");
    assert_eq!(from.type_params, vec![type_param("T")]);
    assert_eq!(
        from.params,
        vec![param("values", applied("Iterable", vec![ty("T")]))]
    );
    assert_eq!(from.return_type, Some(applied("Array", vec![ty("T")])));
    assert_eq!(from.visibility, Visibility::Default);
    assert!(!from.getter);
    assert!(from.body.is_some());
}

#[test]
fn print_contracts_render_any_value_and_aliases_match() {
    let program = program();
    let printer = type_decl(&program, "Printer", TypeKind::Interface);
    let os = type_decl(&program, "OS", TypeKind::Object);
    for method in members(printer) {
        let last = method.params.last().unwrap();
        assert!(last.variadic);
        assert_eq!(last.ty, Some(vector(ty("Any"))));
        let alias = function_decl(&program, &method.name);
        assert_eq!(alias.params, method.params);
        let os_methods = members(os);
        let os_method = os_methods
            .iter()
            .find(|candidate| candidate.name == method.name)
            .unwrap();
        assert_eq!(os_method.params, method.params);
    }
    let any = type_decl(&program, "Any", TypeKind::Interface);
    let methods = members(any);
    assert_eq!(
        methods
            .iter()
            .map(|method| method.name.as_str())
            .collect::<Vec<_>>(),
        vec!["toStr", "equals"]
    );
    assert_eq!(methods[0].return_type, Some(ty("Str")));
    assert_eq!(methods[1].params[0].ty, Some(ty("Any")));
}

#[test]
fn runtime_owned_classes_cannot_be_constructed_in_source() {
    let program = program();
    let regex = module_program("lume/regex").unwrap();
    for (program, name) in [
        (&program, "ReferenceId"),
        (&program, "FileStream"),
        (&program, "TextFileReader"),
        (&regex, "Regex"),
        (&regex, "RegexMatch"),
    ] {
        let declaration = type_decl(program, name, TypeKind::Class);
        let constructor = members(declaration)
            .into_iter()
            .find(|method| method.name == "new")
            .unwrap();
        assert_eq!(
            constructor.visibility,
            Visibility::Private,
            "{name} constructor"
        );
        assert!(constructor.params.is_empty());
    }
}

#[test]
fn union_payloads_extensions_and_tuple_fields_are_preserved() {
    let program = program();
    for (name, variants) in [
        (
            "Option",
            vec![("Some", vec![("value", "T")]), ("None", vec![])],
        ),
        (
            "Result",
            vec![("Ok", vec![("value", "T")]), ("Err", vec![("error", "E")])],
        ),
        (
            "Either",
            vec![
                ("Left", vec![("value", "L")]),
                ("Right", vec![("value", "R")]),
            ],
        ),
    ] {
        let declaration = type_decl(&program, name, TypeKind::Enum);
        let actual = declaration
            .members
            .iter()
            .map(|member| {
                let TypeMember::Case(case) = member else {
                    panic!("union alternative");
                };
                (
                    case.name.clone(),
                    case.fields
                        .iter()
                        .map(|field| (field.name.clone(), field.ty.clone().unwrap()))
                        .collect::<Vec<_>>(),
                )
            })
            .collect::<Vec<_>>();
        let expected = variants
            .into_iter()
            .map(|(name, fields)| {
                (
                    name.to_string(),
                    fields
                        .into_iter()
                        .map(|(name, field_type)| (name.to_string(), ty(field_type)))
                        .collect::<Vec<_>>(),
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(actual, expected);
    }
    for (name, count) in [("Option", 8), ("Result", 9), ("Either", 11)] {
        assert_eq!(extension(&program, name).methods.len(), count);
    }
    for arity in 2..=10 {
        let tuple = type_decl(&program, &format!("Tuple{arity}"), TypeKind::Class);
        assert_eq!(tuple.type_params.len(), arity);
        assert_eq!(tuple.members.len(), arity);
        for (index, member) in tuple.members.iter().enumerate() {
            let TypeMember::Field(field) = member else {
                panic!("tuple field");
            };
            assert_eq!(field.name, format!("_{}", index + 1));
            assert_eq!(field.ty, Some(ty(&tuple.type_params[index].name)));
        }
    }
}

#[test]
fn native_modules_are_explicit_and_keep_reified_parameters() {
    assert!(module_program("unrelated/module").is_none());
    for (path, expected_types) in [
        (
            "lume/json",
            vec!["JsonName", "JsonIgnore", "JsonField", "JsonValue", "Json"],
        ),
        (
            "lume/regex",
            vec!["RegexError", "RegexMatch", "Regex", "Regex"],
        ),
    ] {
        let program = module_program(path).unwrap();
        assert_eq!(program.module.as_ref().unwrap().name, path);
        assert!(program.imports.is_empty());
        assert_eq!(
            program
                .items
                .iter()
                .filter_map(|item| match item {
                    Item::Type(declaration) => Some(declaration.name.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>(),
            expected_types
        );
    }
    let json = module_program("lume/json").unwrap();
    let decode = members(type_decl(&json, "Json", TypeKind::Object))
        .into_iter()
        .find(|method| method.name == "decode")
        .unwrap();
    assert_eq!(decode.type_params, vec![reified_param("T")]);
    let program = program();
    let annotated = type_decl(&program, "Annotated", TypeKind::Interface);
    for method in members(annotated) {
        assert_eq!(method.type_params, vec![reified_param("Ann")]);
    }
}

#[test]
fn type_names_and_function_names_do_not_accidentally_collide() {
    let program = program();
    let mut types = BTreeMap::new();
    let mut functions = HashSet::new();
    for item in &program.items {
        match item {
            Item::Type(declaration) => {
                let key = (
                    declaration.name.clone(),
                    declaration.kind == TypeKind::Object,
                );
                assert!(
                    types.insert(key, declaration).is_none(),
                    "duplicate type {}",
                    declaration.name
                );
            }
            Item::Function(function) => assert!(
                functions.insert(&function.name),
                "duplicate function {}",
                function.name
            ),
            _ => {}
        }
    }
}
