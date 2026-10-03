use std::{cell::RefCell, rc::Rc};

use crate::{
    Diagnostic, Span,
    ast::TypeKind,
    interpreter::{Interpreter, IteratorState, Value, aggregate_named_field, iterable_values},
    ir,
    runtime::{RuntimeField, RuntimeFieldSlot, RuntimeType, RuntimeTypeId},
};

use super::builtin_method;

pub(super) fn define() -> RuntimeType {
    RuntimeType {
        id: RuntimeTypeId(usize::MAX),
        ir_type_id: None,
        kind: TypeKind::Class,
        name: "IntRange".to_string(),
        fields: vec![field(0, "start"), field(1, "end"), field(2, "step")],
        field_init: None,
        methods: vec![
            builtin_method(0, "iterator", Vec::new(), range_iterator),
            builtin_method(1, "zip", vec![ir::Type::Unknown], range_zip),
            builtin_method(2, "zipWithIndex", Vec::new(), range_zip_with_index),
        ],
        enum_cases: Vec::new(),
        with_bounds: Vec::new(),
    }
}

fn field(slot: usize, name: &str) -> RuntimeField {
    RuntimeField {
        slot: RuntimeFieldSlot(slot),
        name: name.to_string(),
        ty: ir::Type::Int,
        mutable: false,
        hidden: false,
        has_initializer: false,
        initializer: None,
    }
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
    Ok(Value::Iterator(Rc::new(RefCell::new(
        IteratorState::Range {
            current: range_field(interpreter, &receiver, "start", span)?,
            end: range_field(interpreter, &receiver, "end", span)?,
            step: range_field(interpreter, &receiver, "step", span)?,
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
