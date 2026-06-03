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
        name: "Result".to_string(),
        fields: Vec::new(),
        methods: vec![
            builtin_method(0, "isOk", Vec::new(), result_is_ok),
            builtin_method(1, "isErr", Vec::new(), result_is_err),
            builtin_method(
                2,
                "map",
                vec![ir::Type::Function {
                    params: Vec::new(),
                    ret: Box::new(ir::Type::Unknown),
                }],
                result_map,
            ),
            builtin_method(3, "expect", Vec::new(), result_expect),
            builtin_method(4, "getError", Vec::new(), result_get_error),
            builtin_method(5, "getOr", vec![ir::Type::Unknown], result_get_or),
        ],
        enum_cases: vec![
            RuntimeEnumCase {
                id: RuntimeEnumCaseId(0),
                name: "Ok".to_string(),
                fields: vec![RuntimeField {
                    slot: RuntimeFieldSlot(0),
                    name: "value".to_string(),
                    ty: ir::Type::Unknown,
                    mutable: false,
                    initializer: None,
                }],
            },
            RuntimeEnumCase {
                id: RuntimeEnumCaseId(1),
                name: "Err".to_string(),
                fields: vec![RuntimeField {
                    slot: RuntimeFieldSlot(0),
                    name: "error".to_string(),
                    ty: ir::Type::Unknown,
                    mutable: false,
                    initializer: None,
                }],
            },
        ],
        with_bounds: Vec::new(),
    }
}

fn builtin_method(
    slot: usize,
    name: &str,
    params: Vec<ir::Type>,
    target: for<'a> fn(
        &mut Interpreter<'a>,
        Value,
        Vec<Value>,
        Option<Span>,
    ) -> Result<Value, Diagnostic>,
) -> RuntimeMethod {
    RuntimeMethod {
        slot: RuntimeMethodSlot(slot),
        name: name.to_string(),
        target: RuntimeMethodTarget::Builtin(target),
        params,
    }
}

fn result_case(receiver: &Value) -> (String, Option<Value>) {
    let (case_name, fields) = receiver.variant_case_name_and_fields().expect("Result variant");
    (case_name, fields.into_iter().next())
}

fn result_is_ok(
    _interpreter: &mut Interpreter<'_>,
    receiver: Value,
    _args: Vec<Value>,
    _span: Option<Span>,
) -> Result<Value, Diagnostic> {
    let (case_name, _) = result_case(&receiver);
    Ok(Value::Bool(case_name == "Ok"))
}

fn result_is_err(
    _interpreter: &mut Interpreter<'_>,
    receiver: Value,
    _args: Vec<Value>,
    _span: Option<Span>,
) -> Result<Value, Diagnostic> {
    let (case_name, _) = result_case(&receiver);
    Ok(Value::Bool(case_name != "Ok"))
}

fn result_map(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    let [callback] = args.as_slice() else {
        return Err(interpreter.runtime_error(span, "Result.map expects 1 argument"));
    };
    let (case_name, first_field) = result_case(&receiver);
    if case_name == "Ok" {
        let mapped = interpreter.invoke_value(
            callback.clone(),
            vec![first_field.expect("Result.Ok payload")],
            span,
        )?;
        Ok(Value::result_ok(mapped))
    } else {
        Ok(receiver)
    }
}

fn result_expect(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    _args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    let (case_name, first_field) = result_case(&receiver);
    if case_name == "Ok" {
        Ok(first_field.expect("Result.Ok payload"))
    } else {
        Err(interpreter.runtime_error(span, "Result has no success value"))
    }
}

fn result_get_error(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    _args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    let (case_name, first_field) = result_case(&receiver);
    if case_name == "Err" {
        Ok(first_field.expect("Result.Err payload"))
    } else {
        Err(interpreter.runtime_error(span, "Result has no error value"))
    }
}

fn result_get_or(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    if args.len() != 1 {
        return Err(interpreter.runtime_error(span, "Result.getOr expects 1 argument"));
    }
    let (case_name, first_field) = result_case(&receiver);
    if case_name == "Ok" {
        Ok(first_field.expect("Result.Ok payload"))
    } else {
        Ok(args[0].clone())
    }
}
