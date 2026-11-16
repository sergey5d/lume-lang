use super::*;

#[test]
fn contracts_preserve_parameter_names_generics_and_getter_kinds() {
    let map = ir::Type::Named {
        name: "Map".into(),
        args: vec![ir::Type::Str, ir::Type::Int],
    };
    let get = method_signature(&map, "get", 1).unwrap();
    assert_eq!(get.param_names, ["key"]);
    assert_eq!(get.params, [ir::Type::Str]);
    assert_eq!(get.return_ty, ir::Type::option(ir::Type::Int));
    assert!(!get.getter);
    assert!(method_signature(&map, "size", 0).unwrap().getter);

    let vector = ir::Type::list(ir::Type::Str);
    let add = method_signature(&vector, "add", 1).unwrap();
    assert_eq!(add.param_names, ["value"]);
    assert_eq!(add.params, [ir::Type::Str]);
    assert_eq!(add.return_ty, vector);
    let (_, params, ret) = runtime_method_signature("Vector", "map", 1).unwrap();
    assert_eq!(
        params,
        [ir::Type::Function {
            params: vec![ir::Type::TypeParam("T".into())],
            ret: Box::new(ir::Type::TypeParam("X".into())),
        }]
    );
    assert_eq!(ret, ir::Type::list(ir::Type::TypeParam("X".into())));
}

#[test]
fn constructors_defaults_and_companions_are_declared_contracts() {
    let vector = type_declaration("Vector", TypeKind::Class).unwrap();
    let constructor = member_declarations(vector, "new").remove(0);
    assert!(constructor.params[0].variadic);
    assert_eq!(constructor.params[0].name, "values");
    assert!(static_method_signature("Vector", "fill", 2).is_some());
    assert!(static_method_signature("Array", "generate", 2).is_some());
    assert!(static_method_signature("Int", "parse", 1).is_some());
    assert!(method_signature(&ir::Type::Int, "parse", 1).is_none());

    let assert = declarations()
        .items
        .iter()
        .find_map(|item| match item {
            ast::Item::Function(function) if function.name == "assert" => Some(function),
            _ => None,
        })
        .unwrap();
    assert!(assert.params[1].initializer.is_some());
    assert!(!assert.params[1].variadic);

    let method =
        method_contract(&ir::Type::list(ir::Type::list(ir::Type::Int)), "flatten", 0).unwrap();
    assert!(
        matches!(method.generic_conditions.as_slice(), [ir::GenericCondition::Bound { subject, bound }]
        if subject == &ir::Type::list(ir::Type::Int)
            && bound == &ir::Type::Named { name: "Iterable".into(), args: vec![ir::Type::TypeParam("X".into())] })
    );
}

#[test]
fn intrinsic_operations_are_explicit_capabilities_not_universal_methods() {
    for name in ["Int", "Float", "Decimal"] {
        assert!(is_primitive(name));
        assert!(supports_operator(name, BinaryOp::Add));
        assert!(supports_operator(name, BinaryOp::Mul));
        assert!(supports_unary_operator(name, UnaryOp::Neg));
    }
    assert!(supports_operator("Str", BinaryOp::Add));
    assert!(!supports_operator("Str", BinaryOp::Sub));
    assert!(!supports_operator("Bool", BinaryOp::Add));
    assert!(!supports_operator("Vector", BinaryOp::Mul));
    assert!(!is_primitive("User"));
    assert!(
        descriptor("Vector")
            .unwrap()
            .operators
            .contains(&OperatorDescriptor::Iterate)
    );
    assert!(
        descriptor("Option")
            .unwrap()
            .operators
            .contains(&OperatorDescriptor::Extract)
    );
    assert!(
        !descriptor("Str")
            .unwrap()
            .operators
            .contains(&OperatorDescriptor::Extract)
    );
}

#[test]
fn canonical_contracts_have_full_types_and_unique_type_identities() {
    let mut identities = std::collections::HashSet::new();
    for descriptor in descriptors() {
        assert!(
            identities.insert((&descriptor.name, descriptor.kind)),
            "duplicate {}",
            descriptor.name
        );
        for member in &descriptor.declaration.members {
            if let TypeMember::Method(method) = member {
                assert!(
                    method.params.iter().all(|param| param.ty.is_some()),
                    "{}.{} has an untyped parameter",
                    descriptor.name,
                    method.name
                );
                assert!(
                    method.name == "new" || method.return_type.is_some(),
                    "{}.{} has no result type",
                    descriptor.name,
                    method.name
                );
                let variables = descriptor
                    .declaration
                    .type_params
                    .iter()
                    .chain(&method.type_params)
                    .map(|param| param.name.as_str())
                    .collect::<Vec<_>>();
                for param in &method.params {
                    assert!(
                        !contains_unknown(&type_to_ir(param.ty.as_ref().unwrap(), &variables)),
                        "{}.{} has an incomplete contract",
                        descriptor.name,
                        method.name
                    );
                }
            }
        }
    }
    assert!(identities.len() > 35);
}

fn contains_unknown(ty: &ir::Type) -> bool {
    match ty {
        ir::Type::Unknown => true,
        ir::Type::Named { args, .. } | ir::Type::Tuple(args) | ir::Type::Union(args) => {
            args.iter().any(contains_unknown)
        }
        ir::Type::Record(fields) => fields.iter().any(|field| contains_unknown(&field.ty)),
        ir::Type::Function { params, ret } => {
            params.iter().any(contains_unknown) || contains_unknown(ret)
        }
        _ => false,
    }
}
