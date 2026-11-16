use crate::{
    Diagnostic, Span,
    interpreter::{Interpreter, Value},
};

use super::{builtin_method, invoke_callback};
use crate::runtime::RuntimeType;

pub(super) fn define() -> RuntimeType {
    let mut ty = super::empty_builtin_type("Option");
    ty.methods = vec![
        builtin_method("Option", "isSet", 0, option_is_set),
        builtin_method("Option", "isEmpty", 0, option_is_empty),
        builtin_method("Option", "map", 1, option_map),
        builtin_method("Option", "orElse", 1, option_or_else),
        builtin_method("Option", "toResult", 1, option_to_result),
        builtin_method("Option", "toEither", 1, option_to_either),
        builtin_method("Option", "iterator", 0, option_iterator),
        builtin_method("Option", "isSuccess", 0, option_is_set),
        builtin_method("Option", "flatMap", 1, option_flat_map),
    ]
    .into_iter()
    .flatten()
    .collect();
    ty
}

fn option_case(receiver: &Value) -> (bool, Option<Value>) {
    let (case, fields) = receiver
        .variant_case_name_and_fields()
        .expect("Option variant");
    (case == "Some", fields.into_iter().next())
}

fn option_is_set(
    _interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    _span: Option<Span>,
) -> Result<Value, Diagnostic> {
    debug_assert!(args.is_empty());
    let (is_some, _) = option_case(&receiver);
    Ok(Value::Bool(is_some))
}

fn option_is_empty(
    _interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    _span: Option<Span>,
) -> Result<Value, Diagnostic> {
    debug_assert!(args.is_empty());
    let (is_some, _) = option_case(&receiver);
    Ok(Value::Bool(!is_some))
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
    let (is_some, first_field) = option_case(&receiver);
    if is_some {
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

fn option_flat_map(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    let [callback] = args.as_slice() else {
        return Err(interpreter.runtime_error(span, "Option.flatMap expects 1 argument"));
    };
    let (is_some, first_field) = option_case(&receiver);
    if is_some {
        interpreter.invoke_value(
            callback.clone(),
            vec![first_field.expect("Option.Some payload")],
            span,
        )
    } else {
        Ok(interpreter.option_none())
    }
}

fn option_or_else(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    if args.len() != 1 {
        return Err(interpreter.runtime_error(span, "Option.orElse expects 1 argument"));
    }
    let (is_some, _) = option_case(&receiver);
    if is_some {
        Ok(receiver)
    } else {
        invoke_callback(interpreter, args[0].clone(), span)
    }
}

fn option_to_result(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    if args.len() != 1 {
        return Err(interpreter.runtime_error(span, "Option.toResult expects 1 argument"));
    }
    let (is_some, first_field) = option_case(&receiver);
    if is_some {
        Ok(interpreter.result_ok(first_field.expect("Option.Some payload")))
    } else {
        let error = invoke_callback(interpreter, args[0].clone(), span)?;
        Ok(interpreter.result_err(error))
    }
}

fn option_to_either(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    if args.len() != 1 {
        return Err(interpreter.runtime_error(span, "Option.toEither expects 1 argument"));
    }
    let (is_some, first_field) = option_case(&receiver);
    if is_some {
        Ok(interpreter.either_right(first_field.expect("Option.Some payload")))
    } else {
        let left = invoke_callback(interpreter, args[0].clone(), span)?;
        Ok(interpreter.either_left(left))
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
    let (is_some, first_field) = option_case(&receiver);
    let values = if is_some {
        vec![first_field.expect("Option.Some payload")]
    } else {
        Vec::new()
    };
    Ok(Value::iterator_from_values(values))
}
