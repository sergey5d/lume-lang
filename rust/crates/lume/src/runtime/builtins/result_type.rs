use crate::{
    Diagnostic, Span,
    ast::TypeKind,
    interpreter::{Interpreter, Value},
    ir,
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

const OK_CASE: RuntimeEnumCaseId = RuntimeEnumCaseId(0);
const ERR_CASE: RuntimeEnumCaseId = RuntimeEnumCaseId(1);

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

fn result_case(receiver: &Value) -> (RuntimeEnumCaseId, Option<Value>) {
    let (_, case_id, fields) = receiver
        .variant_case_ids_and_fields()
        .expect("Result variant");
    (case_id, fields.into_iter().next())
}

fn result_is_ok(
    _interpreter: &mut Interpreter<'_>,
    receiver: Value,
    _args: Vec<Value>,
    _span: Option<Span>,
) -> Result<Value, Diagnostic> {
    let (case_id, _) = result_case(&receiver);
    Ok(Value::Bool(case_id == OK_CASE))
}

fn result_is_err(
    _interpreter: &mut Interpreter<'_>,
    receiver: Value,
    _args: Vec<Value>,
    _span: Option<Span>,
) -> Result<Value, Diagnostic> {
    let (case_id, _) = result_case(&receiver);
    Ok(Value::Bool(case_id == ERR_CASE))
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
    let (case_id, first_field) = result_case(&receiver);
    if case_id == OK_CASE {
        let mapped = interpreter.invoke_value(
            callback.clone(),
            vec![first_field.expect("Result.Ok payload")],
            span,
        )?;
        Ok(interpreter.result_ok(mapped))
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
    let (case_id, first_field) = result_case(&receiver);
    if case_id == OK_CASE {
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
    let (case_id, first_field) = result_case(&receiver);
    if case_id == ERR_CASE {
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
    let (case_id, first_field) = result_case(&receiver);
    if case_id == OK_CASE {
        Ok(first_field.expect("Result.Ok payload"))
    } else {
        Ok(args[0].clone())
    }
}
