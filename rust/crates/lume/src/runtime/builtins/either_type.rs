use crate::{
    Diagnostic, Span,
    ast::TypeKind,
    interpreter::{Interpreter, Value},
    ir,
};

use super::{builtin_method, force_lazy_arg};
use crate::runtime::{
    RuntimeEnumCase, RuntimeEnumCaseId, RuntimeField, RuntimeFieldSlot, RuntimeType, RuntimeTypeId,
};

pub(super) fn define() -> RuntimeType {
    RuntimeType {
        id: RuntimeTypeId(usize::MAX),
        ir_type_id: None,
        kind: TypeKind::Enum,
        name: "Either".to_string(),
        fields: Vec::new(),
        field_init: None,
        methods: vec![
            builtin_method(0, "isLeft", Vec::new(), either_is_left),
            builtin_method(1, "isRight", Vec::new(), either_is_right),
            builtin_method(
                2,
                "map",
                vec![ir::Type::Function {
                    params: Vec::new(),
                    ret: Box::new(ir::Type::Unknown),
                }],
                either_map,
            ),
            builtin_method(3, "expectLeft", Vec::new(), either_expect_left),
            builtin_method(4, "expectRight", Vec::new(), either_expect_right),
            builtin_method(5, "getOr", vec![ir::Type::Unknown], either_get_or),
            builtin_method(6, "orElse", vec![ir::Type::Unknown], either_or_else),
            builtin_method(7, "isSuccess", Vec::new(), either_is_right),
            builtin_method(
                8,
                "flatMap",
                vec![ir::Type::Function {
                    params: Vec::new(),
                    ret: Box::new(ir::Type::Unknown),
                }],
                either_flat_map,
            ),
            builtin_method(
                9,
                "mapLeft",
                vec![ir::Type::Function {
                    params: Vec::new(),
                    ret: Box::new(ir::Type::Unknown),
                }],
                either_map_left,
            ),
            builtin_method(10, "toOption", Vec::new(), either_to_option),
            builtin_method(11, "toResult", Vec::new(), either_to_result),
            builtin_method(12, "merge", Vec::new(), either_merge),
        ],
        enum_cases: vec![
            RuntimeEnumCase {
                id: RuntimeEnumCaseId(0),
                name: "Left".to_string(),
                fields: vec![RuntimeField {
                    slot: RuntimeFieldSlot(0),
                    name: "value".to_string(),
                    ty: ir::Type::Unknown,
                    mutable: false,
                    hidden: false,
                    has_initializer: false,
                    initializer: None,
                }],
            },
            RuntimeEnumCase {
                id: RuntimeEnumCaseId(1),
                name: "Right".to_string(),
                fields: vec![RuntimeField {
                    slot: RuntimeFieldSlot(0),
                    name: "value".to_string(),
                    ty: ir::Type::Unknown,
                    mutable: false,
                    hidden: false,
                    has_initializer: false,
                    initializer: None,
                }],
            },
        ],
        with_bounds: Vec::new(),
    }
}

const LEFT_CASE: RuntimeEnumCaseId = RuntimeEnumCaseId(0);
const RIGHT_CASE: RuntimeEnumCaseId = RuntimeEnumCaseId(1);

fn either_case(receiver: &Value) -> (RuntimeEnumCaseId, Option<Value>) {
    let (_, case_id, fields) = receiver
        .variant_case_ids_and_fields()
        .expect("Either variant");
    (case_id, fields.into_iter().next())
}

fn either_is_left(
    _interpreter: &mut Interpreter<'_>,
    receiver: Value,
    _args: Vec<Value>,
    _span: Option<Span>,
) -> Result<Value, Diagnostic> {
    let (case_id, _) = either_case(&receiver);
    Ok(Value::Bool(case_id == LEFT_CASE))
}

fn either_is_right(
    _interpreter: &mut Interpreter<'_>,
    receiver: Value,
    _args: Vec<Value>,
    _span: Option<Span>,
) -> Result<Value, Diagnostic> {
    let (case_id, _) = either_case(&receiver);
    Ok(Value::Bool(case_id == RIGHT_CASE))
}

fn either_map(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    let [callback] = args.as_slice() else {
        return Err(interpreter.runtime_error(span, "Either.map expects 1 argument"));
    };
    let (case_id, first_field) = either_case(&receiver);
    if case_id == RIGHT_CASE {
        let mapped = interpreter.invoke_value(
            callback.clone(),
            vec![first_field.expect("Either.Right payload")],
            span,
        )?;
        Ok(interpreter.either_right(mapped))
    } else {
        Ok(receiver)
    }
}

fn either_flat_map(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    let [callback] = args.as_slice() else {
        return Err(interpreter.runtime_error(span, "Either.flatMap expects 1 argument"));
    };
    let (case_id, first_field) = either_case(&receiver);
    if case_id == RIGHT_CASE {
        interpreter.invoke_value(
            callback.clone(),
            vec![first_field.expect("Either.Right payload")],
            span,
        )
    } else {
        Ok(receiver)
    }
}

fn either_map_left(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    let [callback] = args.as_slice() else {
        return Err(interpreter.runtime_error(span, "Either.mapLeft expects 1 argument"));
    };
    let (case_id, first_field) = either_case(&receiver);
    if case_id == LEFT_CASE {
        let mapped = interpreter.invoke_value(
            callback.clone(),
            vec![first_field.expect("Either.Left payload")],
            span,
        )?;
        Ok(interpreter.either_left(mapped))
    } else {
        Ok(receiver)
    }
}

fn either_expect_left(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    _args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    let (case_id, first_field) = either_case(&receiver);
    if case_id == LEFT_CASE {
        Ok(first_field.expect("Either.Left payload"))
    } else {
        Err(interpreter.runtime_error(span, "Either has no left value"))
    }
}

fn either_expect_right(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    _args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    let (case_id, first_field) = either_case(&receiver);
    if case_id == RIGHT_CASE {
        Ok(first_field.expect("Either.Right payload"))
    } else {
        Err(interpreter.runtime_error(span, "Either has no right value"))
    }
}

fn either_get_or(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    if args.len() != 1 {
        return Err(interpreter.runtime_error(span, "Either.getOr expects 1 argument"));
    }
    let (case_id, first_field) = either_case(&receiver);
    if case_id == RIGHT_CASE {
        Ok(first_field.expect("Either.Right payload"))
    } else {
        force_lazy_arg(interpreter, args[0].clone(), span)
    }
}

fn either_or_else(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    if args.len() != 1 {
        return Err(interpreter.runtime_error(span, "Either.orElse expects 1 argument"));
    }
    let (case_id, _) = either_case(&receiver);
    if case_id == RIGHT_CASE {
        Ok(receiver)
    } else {
        force_lazy_arg(interpreter, args[0].clone(), span)
    }
}

fn either_to_option(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    if !args.is_empty() {
        return Err(interpreter.runtime_error(span, "Either.toOption expects 0 arguments"));
    }
    let (case_id, first_field) = either_case(&receiver);
    if case_id == RIGHT_CASE {
        Ok(interpreter.option_some(first_field.expect("Either.Right payload")))
    } else {
        Ok(interpreter.option_none())
    }
}

fn either_to_result(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    if !args.is_empty() {
        return Err(interpreter.runtime_error(span, "Either.toResult expects 0 arguments"));
    }
    let (case_id, first_field) = either_case(&receiver);
    if case_id == RIGHT_CASE {
        Ok(interpreter.result_ok(first_field.expect("Either.Right payload")))
    } else {
        Ok(interpreter.result_err(first_field.expect("Either.Left payload")))
    }
}

fn either_merge(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    if !args.is_empty() {
        return Err(interpreter.runtime_error(span, "Either.merge expects 0 arguments"));
    }
    let (_, first_field) = either_case(&receiver);
    Ok(first_field.expect("Either payload"))
}
