use crate::{
    Diagnostic, Span,
    ast::TypeKind,
    interpreter::{Interpreter, Value},
    ir,
};

use super::builtin_method;
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
            builtin_method(6, "isSuccess", Vec::new(), either_is_right),
            builtin_method(7, "unwrap", Vec::new(), either_expect_right),
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
        Ok(args[0].clone())
    }
}
