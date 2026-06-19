use crate::{
    ast::TypeKind,
    interpreter::{Interpreter, Value},
    ir, Diagnostic, Span,
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
        name: "Option".to_string(),
        fields: Vec::new(),
        field_init: None,
        methods: vec![
            builtin_method(0, "isSet", Vec::new(), option_is_set),
            builtin_method(1, "isEmpty", Vec::new(), option_is_empty),
            builtin_method(
                2,
                "map",
                vec![ir::Type::Function {
                    params: Vec::new(),
                    ret: Box::new(ir::Type::Unknown),
                }],
                option_map,
            ),
            builtin_method(3, "orPanic", Vec::new(), option_or_panic),
            builtin_method(4, "getOr", vec![ir::Type::Unknown], option_get_or),
            builtin_method(5, "getOrElse", vec![ir::Type::Unknown], option_get_or_else),
            builtin_method(6, "iterator", Vec::new(), option_iterator),
            builtin_method(7, "isSuccess", Vec::new(), option_is_set),
            builtin_method(8, "unwrap", Vec::new(), option_or_panic),
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
                    hidden: false,
                    has_initializer: false,
                    initializer: None,
                }],
            },
        ],
        with_bounds: Vec::new(),
    }
}

const NONE_CASE: RuntimeEnumCaseId = RuntimeEnumCaseId(0);
const SOME_CASE: RuntimeEnumCaseId = RuntimeEnumCaseId(1);

fn option_case(receiver: &Value) -> (RuntimeEnumCaseId, Option<Value>) {
    let (_, case_id, fields) = receiver
        .variant_case_ids_and_fields()
        .expect("Option variant");
    (case_id, fields.into_iter().next())
}

fn option_is_set(
    _interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    _span: Option<Span>,
) -> Result<Value, Diagnostic> {
    debug_assert!(args.is_empty());
    let (case_id, _) = option_case(&receiver);
    Ok(Value::Bool(case_id == SOME_CASE))
}

fn option_is_empty(
    _interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    _span: Option<Span>,
) -> Result<Value, Diagnostic> {
    debug_assert!(args.is_empty());
    let (case_id, _) = option_case(&receiver);
    Ok(Value::Bool(case_id == NONE_CASE))
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
    let (case_id, first_field) = option_case(&receiver);
    if case_id == SOME_CASE {
        let mapped = interpreter.invoke_value(
            callback.clone(),
            vec![first_field.expect("Option.Some payload")],
            span,
        )?;
        Ok(interpreter.option_some(mapped))
    } else {
        Ok(interpreter.option_none())
    }
}

fn option_or_panic(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    if !args.is_empty() {
        return Err(interpreter.runtime_error(span, "Option.orPanic expects 0 arguments"));
    }
    let (case_id, first_field) = option_case(&receiver);
    if case_id != SOME_CASE {
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
    let (case_id, first_field) = option_case(&receiver);
    if case_id == SOME_CASE {
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
    let (case_id, first_field) = option_case(&receiver);
    if case_id == SOME_CASE {
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
    let (case_id, first_field) = option_case(&receiver);
    let values = if case_id == SOME_CASE {
        vec![first_field.expect("Option.Some payload")]
    } else {
        Vec::new()
    };
    Ok(Value::iterator_from_values(values))
}
