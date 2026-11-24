use crate::{
    Diagnostic, Span,
    interpreter::{Interpreter, Value},
};

use super::{builtin_method, invoke_callback};
use crate::runtime::RuntimeType;

pub(super) fn define() -> RuntimeType {
    let mut ty = super::empty_builtin_type("Result");
    ty.methods = vec![
        builtin_method("Result", "isOk", 0, result_is_ok),
        builtin_method("Result", "isErr", 0, result_is_err),
        builtin_method("Result", "map", 1, result_map),
        builtin_method("Result", "expectErr", 0, result_expect_err),
        builtin_method("Result", "orElse", 1, result_or_else),
        builtin_method("Result", "flatMap", 1, result_flat_map),
        builtin_method("Result", "mapError", 1, result_map_error),
        builtin_method("Result", "toOption", 0, result_to_option),
        builtin_method("Result", "toEither", 0, result_to_either),
    ]
    .into_iter()
    .flatten()
    .collect();
    ty
}

fn result_case(receiver: &Value) -> (bool, Option<Value>) {
    let (case, fields) = receiver
        .variant_case_name_and_fields()
        .expect("Result variant");
    (case == "Ok", fields.into_iter().next())
}

fn result_is_ok(
    _interpreter: &mut Interpreter<'_>,
    receiver: Value,
    _args: Vec<Value>,
    _span: Option<Span>,
) -> Result<Value, Diagnostic> {
    let (is_ok, _) = result_case(&receiver);
    Ok(Value::Bool(is_ok))
}

fn result_is_err(
    _interpreter: &mut Interpreter<'_>,
    receiver: Value,
    _args: Vec<Value>,
    _span: Option<Span>,
) -> Result<Value, Diagnostic> {
    let (is_ok, _) = result_case(&receiver);
    Ok(Value::Bool(!is_ok))
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
    let (is_ok, first_field) = result_case(&receiver);
    if is_ok {
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

fn result_flat_map(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    let [callback] = args.as_slice() else {
        return Err(interpreter.runtime_error(span, "Result.flatMap expects 1 argument"));
    };
    let (is_ok, first_field) = result_case(&receiver);
    if is_ok {
        interpreter.invoke_value(
            callback.clone(),
            vec![first_field.expect("Result.Ok payload")],
            span,
        )
    } else {
        Ok(receiver)
    }
}

fn result_map_error(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    let [callback] = args.as_slice() else {
        return Err(interpreter.runtime_error(span, "Result.mapError expects 1 argument"));
    };
    let (is_ok, first_field) = result_case(&receiver);
    if !is_ok {
        let mapped = interpreter.invoke_value(
            callback.clone(),
            vec![first_field.expect("Result.Err payload")],
            span,
        )?;
        Ok(interpreter.result_err(mapped))
    } else {
        Ok(receiver)
    }
}

fn result_expect_err(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    _args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    let (is_ok, first_field) = result_case(&receiver);
    if !is_ok {
        Ok(first_field.expect("Result.Err payload"))
    } else {
        Err(interpreter.runtime_error(span, "expected Result.Err"))
    }
}

fn result_or_else(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    if args.len() != 1 {
        return Err(interpreter.runtime_error(span, "Result.orElse expects 1 argument"));
    }
    let (is_ok, _) = result_case(&receiver);
    if is_ok {
        Ok(receiver)
    } else {
        invoke_callback(interpreter, args[0].clone(), span)
    }
}

fn result_to_option(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    if !args.is_empty() {
        return Err(interpreter.runtime_error(span, "Result.toOption expects 0 arguments"));
    }
    let (is_ok, first_field) = result_case(&receiver);
    if is_ok {
        Ok(interpreter.option_some(first_field.expect("Result.Ok payload")))
    } else {
        Ok(interpreter.option_none())
    }
}

fn result_to_either(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    if !args.is_empty() {
        return Err(interpreter.runtime_error(span, "Result.toEither expects 0 arguments"));
    }
    let (is_ok, first_field) = result_case(&receiver);
    if is_ok {
        Ok(interpreter.either_right(first_field.expect("Result.Ok payload")))
    } else {
        Ok(interpreter.either_left(first_field.expect("Result.Err payload")))
    }
}
