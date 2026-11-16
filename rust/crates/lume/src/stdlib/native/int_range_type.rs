use std::{cell::RefCell, rc::Rc};

use crate::{
    Diagnostic, Span,
    interpreter::{Interpreter, IteratorState, Value, aggregate_named_field, iterable_values},
    runtime::RuntimeType,
};

use super::builtin_method;

pub(super) fn define() -> RuntimeType {
    let mut ty = super::empty_builtin_type("IntRange");
    ty.methods = vec![
        builtin_method("IntRange", "iterator", 0, range_iterator),
        builtin_method("IntRange", "zip", 1, range_zip),
        builtin_method("IntRange", "zipWithIndex", 0, range_zip_with_index),
    ]
    .into_iter()
    .flatten()
    .collect();
    ty
}

fn range_field(
    interpreter: &Interpreter<'_>,
    receiver: &Value,
    name: &str,
    span: Option<Span>,
) -> Result<i64, Diagnostic> {
    let Value::Aggregate(range) = receiver else {
        unreachable!("IntRange method receiver must be an aggregate");
    };
    let range = range.borrow();
    let value = aggregate_named_field(&range, name).ok_or_else(|| {
        interpreter.runtime_error(span, format!("IntRange has no field '{name}'"))
    })?;
    value.as_int(interpreter, span, &format!("IntRange.{name}"))
}

fn range_iterator(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    debug_assert!(args.is_empty());
    let step = range_field(interpreter, &receiver, "step", span)?;
    if step == 0 {
        return Err(interpreter.runtime_error(span, "Range step cannot be zero"));
    }
    Ok(Value::Iterator(Rc::new(RefCell::new(
        IteratorState::Range {
            current: range_field(interpreter, &receiver, "start", span)?,
            end: range_field(interpreter, &receiver, "end", span)?,
            step,
        },
    ))))
}

fn range_zip(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    let [other] = args.as_slice() else {
        return Err(interpreter.runtime_error(span, "IntRange.zip expects 1 argument"));
    };
    let left = iterable_values(receiver, span, interpreter)?;
    let right = iterable_values(other.clone(), span, interpreter)?;
    Ok(Value::list(
        left.into_iter()
            .zip(right)
            .map(|(left, right)| Value::Tuple(vec![left, right]))
            .collect(),
    ))
}

fn range_zip_with_index(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    debug_assert!(args.is_empty());
    Ok(Value::list(
        iterable_values(receiver, span, interpreter)?
            .into_iter()
            .enumerate()
            .map(|(index, value)| Value::Tuple(vec![value, Value::Int(index as i64)]))
            .collect(),
    ))
}
