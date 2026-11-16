use super::*;

fn invoke_static(
    interpreter: &mut Interpreter<'_>,
    owner: &str,
    name: &str,
    args: Vec<Value>,
) -> Value {
    interpreter
        .invoke_named_path(None, &[owner.into(), name.into()], args, None, false)
        .unwrap()
}

fn payload(value: Value) -> Value {
    let Value::Aggregate(value) = value else {
        panic!("expected lifted value");
    };
    value
        .borrow()
        .fields
        .first()
        .cloned()
        .expect("lifted payload")
}

#[test]
fn native_registry_routes_primitives_factories_and_root_functions() {
    let program = ir::Program::default();
    let mut interpreter = Interpreter::new(&program, &["first".into()]);
    assert_eq!(
        interpreter
            .invoke_method(Value::Int(2), "pow", vec![Value::Int(10)], None)
            .unwrap()
            .render(),
        "1024"
    );
    assert_eq!(
        interpreter
            .invoke_method(
                Value::Int(i64::MAX),
                "checkedAdd",
                vec![Value::Int(1)],
                None
            )
            .unwrap()
            .render(),
        "None"
    );
    assert_eq!(
        interpreter
            .invoke_method(Value::Float(9.0), "sqrt", vec![], None)
            .unwrap()
            .render(),
        "3.0"
    );
    let parsed = payload(invoke_static(
        &mut interpreter,
        "Decimal",
        "parse",
        vec![Value::String("12.50".into())],
    ));
    assert_eq!(
        interpreter
            .get_member(parsed, "scale", None)
            .unwrap()
            .render(),
        "2"
    );
    assert_eq!(
        payload(invoke_static(
            &mut interpreter,
            "Int",
            "parse",
            vec![Value::String("42".into())]
        ))
        .render(),
        "42"
    );
    assert_eq!(
        interpreter
            .invoke_method(
                Value::String("a".into()),
                "compareTo",
                vec![Value::String("b".into())],
                None
            )
            .unwrap()
            .render(),
        "-1"
    );
    assert_eq!(
        invoke_static(
            &mut interpreter,
            "Math",
            "max",
            vec![Value::Int(3), Value::Int(7)]
        )
        .render(),
        "7"
    );
    let values = invoke_static(
        &mut interpreter,
        "Vector",
        "fill",
        vec![Value::Int(3), Value::Int(8)],
    );
    assert_eq!(
        interpreter
            .get_member(values, "size", None)
            .unwrap()
            .render(),
        "3"
    );
    let ctor = crate::stdlib::native::constructor("Vector").unwrap();
    assert!(matches!(
        ctor(&mut interpreter, Value::Unit, vec![Value::Int(1)], None).unwrap(),
        Value::List(_)
    ));
    let print = crate::stdlib::native::root_function("println").unwrap();
    print(
        &mut interpreter,
        Value::Unit,
        vec![Value::Int(7), Value::Bool(true)],
        None,
    )
    .unwrap();
    assert_eq!(interpreter.output, "7 true\n");
    assert_eq!(
        crate::stdlib::native::root_function("identity").unwrap()(
            &mut interpreter,
            Value::Unit,
            vec![Value::Int(9)],
            None
        )
        .unwrap()
        .render(),
        "9"
    );
}

#[test]
fn native_registry_routes_metadata_json_and_regex() {
    let program = ir::Program::default();
    let mut interpreter = Interpreter::new(&program, &[]);
    let target = interpreter.runtime_type_value_for_ir_type(&ir::Type::Named {
        name: "Vector".into(),
        args: vec![ir::Type::Int],
    });
    let decoded = payload(invoke_static(
        &mut interpreter,
        "Json",
        "decode",
        vec![Value::String("[1,2,3]".into()), Value::RuntimeType(target)],
    ));
    assert_eq!(
        interpreter
            .get_member(decoded.clone(), "size", None)
            .unwrap()
            .render(),
        "3"
    );
    assert_eq!(
        invoke_static(&mut interpreter, "Json", "stringify", vec![decoded]).render(),
        "[1,2,3]"
    );
    let regex = payload(invoke_static(
        &mut interpreter,
        "Regex",
        "compile",
        vec![Value::String("(?P<letter>é)(x)?".into())],
    ));
    let hit = payload(
        interpreter
            .invoke_method(regex, "find", vec![Value::String("aé".into())], None)
            .unwrap(),
    );
    assert_eq!(
        interpreter
            .get_member(hit.clone(), "start", None)
            .unwrap()
            .render(),
        "1"
    );
    assert_eq!(
        interpreter
            .get_member(hit.clone(), "end", None)
            .unwrap()
            .render(),
        "2"
    );
    assert_eq!(
        payload(
            interpreter
                .invoke_method(
                    hit.clone(),
                    "group",
                    vec![Value::String("letter".into())],
                    None
                )
                .unwrap()
        )
        .render(),
        "é"
    );
    let groups = interpreter.get_member(hit.clone(), "groups", None).unwrap();
    interpreter
        .invoke_method(groups, "add", vec![Value::Unit], None)
        .unwrap();
    let groups = interpreter.get_member(hit, "groups", None).unwrap();
    assert_eq!(
        interpreter
            .get_member(groups, "size", None)
            .unwrap()
            .render(),
        "3"
    );
    let type_id = interpreter
        .runtime
        .type_id_by_name_any_kind("Vector")
        .unwrap();
    let metadata = Value::RuntimeType(RuntimeTypeValue::Runtime {
        id: type_id,
        args: vec![],
    });
    let method = payload(
        interpreter
            .invoke_method(metadata, "method", vec![Value::String("add".into())], None)
            .unwrap(),
    );
    assert_eq!(
        interpreter
            .get_member(method, "name", None)
            .unwrap()
            .render(),
        "add"
    );
}

#[test]
fn native_registry_preserves_primitive_extensions_ids_and_bounds() {
    let mut program = ir::Program::default();
    let owner = program.add_type(ir::TypeDef::new(ast::TypeKind::Class, "Int"));
    let mut extension = ir::Function::new(
        "extensionValue",
        ir::FunctionKind::Method { owner },
        ir::Type::Int,
    );
    extension.add_local("this", ir::Type::Int, false, ir::LocalKind::Capture);
    extension.blocks[0].terminator =
        ir::Terminator::ret(Some(ir::Operand::Const(ir::Constant::Int(17))));
    let method = program.add_function(extension);
    program.types[owner.0].methods.push(method);
    let mut interpreter = Interpreter::new(&program, &[]);
    let native = interpreter
        .runtime
        .type_id_by_name_kind("Int", ast::TypeKind::Interface)
        .unwrap();
    assert_eq!(interpreter.runtime.type_id_for_ir_id(owner), Some(native));
    assert_eq!(
        interpreter
            .runtime
            .types
            .iter()
            .filter(|ty| ty.name == "Int" && ty.kind != ast::TypeKind::Object)
            .count(),
        1
    );
    assert!(
        !interpreter
            .runtime
            .type_by_id(native)
            .unwrap()
            .with_bounds
            .is_empty()
    );
    assert_eq!(
        interpreter
            .invoke_method(Value::Int(1), "extensionValue", vec![], None)
            .unwrap()
            .render(),
        "17"
    );
    assert_eq!(
        interpreter
            .invoke_method(Value::Int(2), "pow", vec![Value::Int(3)], None)
            .unwrap()
            .render(),
        "8"
    );
    for ty in &interpreter.runtime.types {
        for (index, method) in ty.methods.iter().enumerate() {
            assert_eq!(method.slot.0, index, "{}.{}", ty.name, method.name);
        }
    }
}

#[test]
fn native_registry_preserves_os_getters_and_closed_files() {
    let program = ir::Program::default();
    let mut interpreter = Interpreter::new(&program, &["first".into()]);
    let os = interpreter.lookup_singleton("OS", None).unwrap().unwrap();
    let arguments = interpreter.get_member(os.clone(), "args", None).unwrap();
    interpreter
        .invoke_method(arguments, "add", vec![Value::String("local".into())], None)
        .unwrap();
    let fresh = interpreter.get_member(os.clone(), "args", None).unwrap();
    assert_eq!(
        interpreter
            .get_member(fresh, "size", None)
            .unwrap()
            .render(),
        "1"
    );
    let stdout = interpreter.get_member(os.clone(), "stdout", None).unwrap();
    assert!(values_identical(&stdout, &os));
    let stream = Value::FileStream(Rc::new(RefCell::new(FileStreamValue {
        path: "closed".into(),
        file: None,
    })));
    assert_eq!(
        interpreter
            .get_member(stream.clone(), "closed", None)
            .unwrap()
            .render(),
        "true"
    );
    let error = payload(
        interpreter
            .invoke_method(stream.clone(), "read", vec![Value::Int(1)], None)
            .unwrap(),
    );
    assert!(
        matches!(error, Value::Aggregate(value) if value.borrow().case_name.as_deref() == Some("Closed"))
    );
    assert!(interpreter.get_member(stream, "position", None).is_err());
}

#[test]
fn native_registry_preserves_array_specific_contracts() {
    let mut program = ir::Program::default();
    let mut predicate = ir::Function::new("predicate", ir::FunctionKind::Lambda, ir::Type::Bool);
    predicate.add_param("value", ir::Type::Int);
    predicate.blocks[0].terminator =
        ir::Terminator::ret(Some(ir::Operand::Const(ir::Constant::Bool(true))));
    let predicate = program.add_function(predicate);
    let mut caller = ir::Function::new("caller", ir::FunctionKind::TopLevel, ir::Type::Unit);
    let local = caller.add_local(
        "items",
        ir::Type::Named {
            name: "Array".into(),
            args: vec![ir::Type::Int],
        },
        false,
        ir::LocalKind::Temp,
    );
    let caller = program.add_function(caller);
    let items = Value::list(vec![Value::Int(1), Value::Int(2)]);
    let frame = Frame {
        function: caller,
        locals: vec![items.clone()],
        defers: vec![],
    };
    let mut interpreter = Interpreter::new(&program, &[]);
    let callee = |method: &str| ir::Callee::Method {
        receiver: ir::Operand::Copy(Box::new(ir::Place::Local(local))),
        method: method.into(),
    };
    let cloned = interpreter
        .invoke_callee(Some(&frame), &callee("clone"), vec![], None, false)
        .unwrap();
    assert!(!values_identical(&items, &cloned));
    let count = interpreter
        .invoke_callee(
            Some(&frame),
            &callee("count"),
            vec![Value::Closure(Rc::new(ClosureValue {
                function: predicate,
                captures: vec![],
            }))],
            None,
            false,
        )
        .unwrap();
    assert_eq!(count.render(), "2");
    assert_eq!(
        payload(
            interpreter
                .invoke_callee(
                    Some(&frame),
                    &callee("find"),
                    vec![Value::Int(2)],
                    None,
                    false
                )
                .unwrap()
        )
        .render(),
        "2"
    );
    assert!(
        interpreter
            .runtime
            .type_by_name_kind("Vector", ast::TypeKind::Class)
            .unwrap()
            .methods
            .iter()
            .all(|method| !matches!(method.name.as_str(), "clone" | "count" | "find"))
    );
}

#[test]
fn native_registry_does_not_capture_source_types_with_native_names() {
    let mut program = ir::Program::default();
    let mut ty = ir::TypeDef::new(ast::TypeKind::Class, "Vector");
    ty.fields.push(ir::Field {
        annotations: vec![],
        visibility: ast::Visibility::Default,
        mutable: false,
        from_mixin: false,
        name: "value".into(),
        ty: ir::Type::Int,
        has_initializer: false,
        initializer: None,
        span: None,
    });
    let source = program.add_type(ty);
    let mut interpreter = Interpreter::new(&program, &[]);
    let id = interpreter.runtime.type_id_for_ir_id(source).unwrap();
    assert!(!interpreter.runtime.type_by_id(id).unwrap().native);
    let constructed = interpreter
        .invoke_root_named("Vector", vec![Value::Int(23)], None, false)
        .unwrap();
    assert!(matches!(constructed, Value::Aggregate(_)));
    assert_eq!(
        interpreter
            .get_member(constructed.clone(), "value", None)
            .unwrap()
            .render(),
        "23"
    );
    assert!(
        interpreter
            .invoke_method(constructed, "add", vec![Value::Int(1)], None)
            .is_err()
    );
    let metadata = interpreter
        .get_member(Value::Int(3), "runtimeType", None)
        .unwrap();
    assert_eq!(
        payload(interpreter.get_member(metadata, "name", None).unwrap()).render(),
        "Int"
    );
    assert!(matches!(
        interpreter
            .invoke_method(Value::Int(3), "hash", vec![], None)
            .unwrap(),
        Value::Int(_)
    ));
}

#[test]
fn native_registry_never_executes_contract_marker_bodies() {
    let mut program = ir::Program::default();
    let declaration = crate::stdlib::type_declaration("Method", ast::TypeKind::Interface).unwrap();
    let mut ty = ir::TypeDef::new(ast::TypeKind::Interface, "Method");
    ty.span = Some(declaration.span);
    let owner = program.add_type(ty);
    let mut marker = ir::Function::new("call", ir::FunctionKind::Method { owner }, ir::Type::Unit);
    marker.span = Some(declaration.span);
    let marker = program.add_function(marker);
    program.types[owner.0].methods.push(marker);
    let mut identity = ir::Function::new("identity", ir::FunctionKind::TopLevel, ir::Type::Unit);
    identity.span = crate::stdlib::declarations()
        .items
        .iter()
        .find_map(|item| match item {
            ast::Item::Function(function) if function.name == "identity" => Some(function.span),
            _ => None,
        });
    let identity = program.add_function(identity);
    let mut interpreter = Interpreter::new(&program, &[]);
    assert_eq!(
        interpreter
            .call_function(identity, None, None, vec![Value::Int(31)], None)
            .unwrap()
            .render(),
        "31"
    );
    let vector = interpreter
        .runtime
        .native_type_by_name_kind("Vector", ast::TypeKind::Class)
        .unwrap();
    let add = vector
        .methods
        .iter()
        .find(|method| method.name == "add")
        .unwrap();
    let metadata = Value::RuntimeMethod {
        owner: vector.id,
        slot: add.slot,
    };
    assert!(
        interpreter
            .invoke_method(metadata, "call", vec![Value::Unit], None)
            .is_err()
    );
    let some = interpreter.option_some(Value::Int(1));
    assert_eq!(
        interpreter
            .get_member(some, "isSet", None)
            .unwrap()
            .render(),
        "true"
    );
    let none = interpreter.option_none();
    assert_eq!(
        interpreter
            .get_member(none, "isEmpty", None)
            .unwrap()
            .render(),
        "true"
    );
}
