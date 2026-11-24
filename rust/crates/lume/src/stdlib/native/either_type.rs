use crate::{
    Diagnostic, Span,
    interpreter::{Interpreter, Value},
};

use super::{builtin_method, invoke_callback};
use crate::runtime::RuntimeType;

pub(super) fn define() -> RuntimeType {
    let mut ty = super::empty_builtin_type("Either");
    ty.methods = vec![
        builtin_method("Either", "isLeft", 0, either_is_left),
        builtin_method("Either", "isRight", 0, either_is_right),
        builtin_method("Either", "map", 1, either_map),
        builtin_method("Either", "expectLeft", 0, either_expect_left),
        builtin_method("Either", "expectRight", 0, either_expect_right),
        builtin_method("Either", "orElse", 1, either_or_else),
        builtin_method("Either", "flatMap", 1, either_flat_map),
        builtin_method("Either", "mapLeft", 1, either_map_left),
        builtin_method("Either", "toOption", 0, either_to_option),
        builtin_method("Either", "toResult", 0, either_to_result),
        builtin_method("Either", "merge", 0, either_merge),
    ]
    .into_iter()
    .flatten()
    .collect();
    ty
}

fn either_case(receiver: &Value) -> (bool, Option<Value>) {
    let (case, fields) = receiver
        .variant_case_name_and_fields()
        .expect("Either variant");
    (case == "Right", fields.into_iter().next())
}

fn either_is_left(
    _interpreter: &mut Interpreter<'_>,
    receiver: Value,
    _args: Vec<Value>,
    _span: Option<Span>,
) -> Result<Value, Diagnostic> {
    let (is_right, _) = either_case(&receiver);
    Ok(Value::Bool(!is_right))
}

fn either_is_right(
    _interpreter: &mut Interpreter<'_>,
    receiver: Value,
    _args: Vec<Value>,
    _span: Option<Span>,
) -> Result<Value, Diagnostic> {
    let (is_right, _) = either_case(&receiver);
    Ok(Value::Bool(is_right))
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
    let (is_right, first_field) = either_case(&receiver);
    if is_right {
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
    let (is_right, first_field) = either_case(&receiver);
    if is_right {
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
    let (is_right, first_field) = either_case(&receiver);
    if !is_right {
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
    let (is_right, first_field) = either_case(&receiver);
    if !is_right {
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
    let (is_right, first_field) = either_case(&receiver);
    if is_right {
        Ok(first_field.expect("Either.Right payload"))
    } else {
        Err(interpreter.runtime_error(span, "Either has no right value"))
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
    let (is_right, _) = either_case(&receiver);
    if is_right {
        Ok(receiver)
    } else {
        invoke_callback(interpreter, args[0].clone(), span)
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
    let (is_right, first_field) = either_case(&receiver);
    if is_right {
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
    let (is_right, first_field) = either_case(&receiver);
    if is_right {
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
