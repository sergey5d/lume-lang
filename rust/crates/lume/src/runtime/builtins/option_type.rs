use crate::{
    ast::TypeKind,
    interpreter::{Interpreter, Value},
    ir,
    Diagnostic, Span,
};

use crate::runtime::{
    RuntimeEnumCase, RuntimeEnumCaseId, RuntimeField, RuntimeFieldSlot, RuntimeMethod,
    RuntimeMethodSlot, RuntimeMethodTarget, RuntimeType, RuntimeTypeId,
};

pub(super) fn define() -> RuntimeType {
    RuntimeType {
        id: RuntimeTypeId(usize::MAX),
        ir_type_id: None,
        kind: TypeKind::Enum,
        name: "Option".to_string(),
        fields: Vec::new(),
        methods: vec![
            RuntimeMethod {
                slot: RuntimeMethodSlot(0),
                name: "isSet".to_string(),
                target: RuntimeMethodTarget::Builtin(option_is_set),
                params: Vec::new(),
            },
            RuntimeMethod {
                slot: RuntimeMethodSlot(1),
                name: "isEmpty".to_string(),
                target: RuntimeMethodTarget::Builtin(option_is_empty),
                params: Vec::new(),
            },
            RuntimeMethod {
                slot: RuntimeMethodSlot(2),
                name: "map".to_string(),
                target: RuntimeMethodTarget::Builtin(option_map),
                params: vec![ir::Type::Function {
                    params: Vec::new(),
                    ret: Box::new(ir::Type::Unknown),
                }],
            },
            RuntimeMethod {
                slot: RuntimeMethodSlot(3),
                name: "expect".to_string(),
                target: RuntimeMethodTarget::Builtin(option_expect),
                params: Vec::new(),
            },
            RuntimeMethod {
                slot: RuntimeMethodSlot(4),
                name: "getOr".to_string(),
                target: RuntimeMethodTarget::Builtin(option_get_or),
                params: vec![ir::Type::Unknown],
            },
            RuntimeMethod {
                slot: RuntimeMethodSlot(5),
                name: "getOrElse".to_string(),
                target: RuntimeMethodTarget::Builtin(option_get_or_else),
                params: vec![ir::Type::Unknown],
            },
            RuntimeMethod {
                slot: RuntimeMethodSlot(6),
                name: "iterator".to_string(),
                target: RuntimeMethodTarget::Builtin(option_iterator),
                params: Vec::new(),
            },
        ],
        enum_cases: vec![
            RuntimeEnumCase {
                id: RuntimeEnumCaseId(0),
                name: "None".to_string(),
                fields: Vec::new(),
            },
            RuntimeEnumCase {
                id: RuntimeEnumCaseId(1),
                name: "Some".to_string(),
                fields: vec![RuntimeField {
                    slot: RuntimeFieldSlot(0),
                    name: "value".to_string(),
                    ty: ir::Type::Unknown,
                    mutable: false,
                    initializer: None,
                }],
            },
        ],
        with_bounds: Vec::new(),
    }
}

fn option_case(receiver: &Value) -> (String, Option<Value>) {
    let (case_name, fields) = receiver.variant_case_name_and_fields().expect("Option variant");
    (case_name, fields.into_iter().next())
}

fn option_is_set(
    _interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    _span: Option<Span>,
) -> Result<Value, Diagnostic> {
    debug_assert!(args.is_empty());
    let (case_name, _) = option_case(&receiver);
    Ok(Value::Bool(case_name == "Some"))
}

fn option_is_empty(
    _interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    _span: Option<Span>,
) -> Result<Value, Diagnostic> {
    debug_assert!(args.is_empty());
    let (case_name, _) = option_case(&receiver);
    Ok(Value::Bool(case_name != "Some"))
}

fn option_map(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    let [callback] = args.as_slice() else {
        return Err(interpreter.runtime_error(span, "Option.map expects 1 argument"));
    };
    let (case_name, first_field) = option_case(&receiver);
    if case_name == "Some" {
        let mapped = interpreter.invoke_value(
            callback.clone(),
            vec![first_field.expect("Option.Some payload")],
            span,
        )?;
        Ok(Value::option_some(mapped))
    } else {
        Ok(Value::option_none())
    }
}

fn option_expect(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    if !args.is_empty() {
        return Err(interpreter.runtime_error(span, "Option.expect expects 0 arguments"));
    }
    let (case_name, first_field) = option_case(&receiver);
    if case_name != "Some" {
        return Err(interpreter.runtime_error(span, "Option has no value"));
    }
    Ok(first_field.expect("Option.Some payload"))
}

fn option_get_or(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    if args.len() != 1 {
        return Err(interpreter.runtime_error(span, "Option.getOr expects 1 argument"));
    }
    let (case_name, first_field) = option_case(&receiver);
    if case_name == "Some" {
        Ok(first_field.expect("Option.Some payload"))
    } else {
        Ok(args[0].clone())
    }
}

fn option_get_or_else(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    if args.len() != 1 {
        return Err(interpreter.runtime_error(span, "Option.getOrElse expects 1 argument"));
    }
    let (case_name, first_field) = option_case(&receiver);
    if case_name == "Some" {
        Ok(first_field.expect("Option.Some payload"))
    } else {
        Ok(args[0].clone())
    }
}

fn option_iterator(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    if !args.is_empty() {
        return Err(interpreter.runtime_error(span, "Option.iterator expects 0 arguments"));
    }
    let (case_name, first_field) = option_case(&receiver);
    let values = if case_name == "Some" {
        vec![first_field.expect("Option.Some payload")]
    } else {
        Vec::new()
    };
    Ok(Value::iterator_from_values(values))
}
