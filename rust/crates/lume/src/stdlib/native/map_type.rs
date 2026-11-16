use std::{cell::RefCell, rc::Rc};

use crate::{
    Diagnostic, Span,
    interpreter::{Interpreter, Value, iterable_values},
};

use super::builtin_method;
use crate::runtime::RuntimeType;

pub(super) fn define() -> RuntimeType {
    let mut ty = super::empty_builtin_type("Map");
    ty.methods = vec![
        builtin_method("Map", "put", 2, map_put),
        builtin_method("Map", "iterator", 0, map_iterator),
        builtin_method("Map", "map", 1, map_map),
        builtin_method("Map", "mapValues", 1, map_map_values),
        builtin_method("Map", "flatMap", 1, map_flat_map),
        builtin_method("Map", "filter", 1, map_filter),
        builtin_method("Map", "fold", 2, map_fold),
        builtin_method("Map", "reduce", 1, map_reduce),
        builtin_method("Map", "exists", 1, map_exists),
        builtin_method("Map", "forAll", 1, map_for_all),
        builtin_method("Map", "forEach", 1, map_for_each),
        builtin_method("Map", "get", 1, map_get),
        builtin_method("Map", "contains", 1, map_contains),
        builtin_method("Map", "size", 0, map_size),
        builtin_method("Map", "entries", 0, map_entries_list),
        builtin_method("Map", "clear", 0, map_clear),
        builtin_method("Map", "values", 0, map_values),
        builtin_method("Map", "keys", 0, map_keys),
        builtin_method("Map", "isEmpty", 0, map_is_empty),
        builtin_method("Map", "nonEmpty", 0, map_non_empty),
    ]
    .into_iter()
    .flatten()
    .collect();
    ty
}

fn map_entries(receiver: &Value) -> Rc<RefCell<Vec<(Value, Value)>>> {
    let Value::Map(entries) = receiver else {
        unreachable!();
    };
    entries.clone()
}

fn map_put(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    if args.len() != 2 {
        return Err(interpreter.runtime_error(span, "Map.put expects 2 arguments"));
    }
    let entries = map_entries(&receiver);
    let snapshot = entries.borrow().clone();
    let existing = interpreter.map_key_index(&snapshot, &args[0], span)?;
    let mut entries = entries.borrow_mut();
    if let Some(position) = existing {
        entries[position].1 = args[1].clone();
    } else {
        entries.push((args[0].clone(), args[1].clone()));
    }
    Ok(receiver)
}

fn map_iterator(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    if !args.is_empty() {
        return Err(interpreter.runtime_error(span, "Map.iterator expects 0 arguments"));
    }
    Ok(Value::iterator_from_values(
        map_entries(&receiver)
            .borrow()
            .iter()
            .map(|(key, value)| Value::Tuple(vec![key.clone(), value.clone()]))
            .collect(),
    ))
}

fn map_entries_list(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    if !args.is_empty() {
        return Err(interpreter.runtime_error(span, "Map.entries expects 0 arguments"));
    }
    Ok(Value::list(
        map_entries(&receiver)
            .borrow()
            .iter()
            .map(|(key, value)| Value::Tuple(vec![key.clone(), value.clone()]))
            .collect(),
    ))
}

fn map_values(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    if !args.is_empty() {
        return Err(interpreter.runtime_error(span, "Map.values expects 0 arguments"));
    }
    Ok(Value::list(
        map_entries(&receiver)
            .borrow()
            .iter()
            .map(|(_, value)| interpreter.clone_value(value))
            .collect(),
    ))
}

fn map_keys(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    if !args.is_empty() {
        return Err(interpreter.runtime_error(span, "Map.keys expects 0 arguments"));
    }
    Ok(Value::list(
        map_entries(&receiver)
            .borrow()
            .iter()
            .map(|(key, _)| interpreter.clone_value(key))
            .collect(),
    ))
}

fn map_clear(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    if !args.is_empty() {
        return Err(interpreter.runtime_error(span, "Map.clear expects 0 arguments"));
    }
    map_entries(&receiver).borrow_mut().clear();
    Ok(Value::Unit)
}

fn map_map(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    let [callback] = args.as_slice() else {
        return Err(interpreter.runtime_error(span, "Map.map expects 1 argument"));
    };
    let pairs = map_entries(&receiver).borrow().clone();
    let mut out = Vec::with_capacity(pairs.len());
    for (key, value) in pairs {
        out.push(interpreter.invoke_value(callback.clone(), vec![key, value], span)?);
    }
    Ok(Value::list(out))
}

fn map_map_values(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    let [callback] = args.as_slice() else {
        return Err(interpreter.runtime_error(span, "Map.mapValues expects 1 argument"));
    };
    let pairs = map_entries(&receiver).borrow().clone();
    let mut out = Vec::with_capacity(pairs.len());
    for (key, value) in pairs {
        let next = interpreter.invoke_value(callback.clone(), vec![value], span)?;
        out.push((key, next));
    }
    Ok(Value::map(out))
}

fn map_flat_map(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    let [callback] = args.as_slice() else {
        return Err(interpreter.runtime_error(span, "Map.flatMap expects 1 argument"));
    };
    let pairs = map_entries(&receiver).borrow().clone();
    let mut out = Vec::new();
    for (key, value) in pairs {
        let mapped = interpreter.invoke_value(callback.clone(), vec![key, value], span)?;
        out.extend(iterable_values(mapped, span, interpreter)?);
    }
    Ok(Value::list(out))
}

fn map_filter(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    let [callback] = args.as_slice() else {
        return Err(interpreter.runtime_error(span, "Map.filter expects 1 argument"));
    };
    let pairs = map_entries(&receiver).borrow().clone();
    let mut out = Vec::new();
    for (key, value) in pairs {
        if interpreter
            .invoke_value(callback.clone(), vec![key.clone(), value.clone()], span)?
            .as_bool(interpreter, span, "Map.filter predicate")?
        {
            out.push((key, value));
        }
    }
    Ok(Value::map(out))
}

fn map_fold(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    if args.len() != 2 {
        return Err(interpreter.runtime_error(span, "Map.fold expects 2 arguments"));
    }
    let mut acc = args[0].clone();
    let callback = args[1].clone();
    let pairs = map_entries(&receiver).borrow().clone();
    for (key, value) in pairs {
        acc = interpreter.invoke_value(callback.clone(), vec![acc, key, value], span)?;
    }
    Ok(acc)
}

fn map_reduce(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    let [callback] = args.as_slice() else {
        return Err(interpreter.runtime_error(span, "Map.reduce expects 1 argument"));
    };
    let pairs = map_entries(&receiver).borrow().clone();
    let Some(((mut left_key, mut left_value), rest)) = pairs
        .split_first()
        .map(|(first, rest)| ((first.0.clone(), first.1.clone()), rest))
    else {
        return Ok(interpreter.option_none());
    };
    for (right_key, right_value) in rest {
        let reduced = interpreter.invoke_value(
            callback.clone(),
            vec![
                left_key.clone(),
                left_value.clone(),
                right_key.clone(),
                right_value.clone(),
            ],
            span,
        )?;
        let Value::Tuple(items) = reduced else {
            return Err(
                interpreter.runtime_error(span, "Map.reduce callback must return a pair tuple")
            );
        };
        if items.len() != 2 {
            return Err(
                interpreter.runtime_error(span, "Map.reduce callback must return a pair tuple")
            );
        }
        left_key = items[0].clone();
        left_value = items[1].clone();
    }
    Ok(interpreter.option_some(Value::Tuple(vec![left_key, left_value])))
}

fn map_exists(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    let [callback] = args.as_slice() else {
        return Err(interpreter.runtime_error(span, "Map.exists expects 1 argument"));
    };
    let pairs = map_entries(&receiver).borrow().clone();
    for (key, value) in pairs {
        if interpreter
            .invoke_value(callback.clone(), vec![key, value], span)?
            .as_bool(interpreter, span, "Map.exists predicate")?
        {
            return Ok(Value::Bool(true));
        }
    }
    Ok(Value::Bool(false))
}

fn map_for_all(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    let [callback] = args.as_slice() else {
        return Err(interpreter.runtime_error(span, "Map.forAll expects 1 argument"));
    };
    let pairs = map_entries(&receiver).borrow().clone();
    for (key, value) in pairs {
        if !interpreter
            .invoke_value(callback.clone(), vec![key, value], span)?
            .as_bool(interpreter, span, "Map.forAll predicate")?
        {
            return Ok(Value::Bool(false));
        }
    }
    Ok(Value::Bool(true))
}

fn map_for_each(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    let [callback] = args.as_slice() else {
        return Err(interpreter.runtime_error(span, "Map.forEach expects 1 argument"));
    };
    let pairs = map_entries(&receiver).borrow().clone();
    for (key, value) in pairs {
        let _ = interpreter.invoke_value(callback.clone(), vec![key, value], span)?;
    }
    Ok(Value::Unit)
}

fn map_get(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    let [needle] = args.as_slice() else {
        return Err(interpreter.runtime_error(span, "Map.get expects 1 argument"));
    };
    let entries = map_entries(&receiver).borrow().clone();
    let found = interpreter
        .map_key_index(&entries, needle, span)?
        .and_then(|position| entries.get(position))
        .map(|(_, value)| value.clone());
    Ok(match found {
        Some(value) => interpreter.option_some(value),
        None => interpreter.option_none(),
    })
}

fn map_contains(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    let [needle] = args.as_slice() else {
        return Err(interpreter.runtime_error(span, "Map.contains expects 1 argument"));
    };
    let entries = map_entries(&receiver).borrow().clone();
    Ok(Value::Bool(
        interpreter.map_key_index(&entries, needle, span)?.is_some(),
    ))
}

fn map_size(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    if !args.is_empty() {
        return Err(interpreter.runtime_error(span, "Map.size expects 0 arguments"));
    }
    Ok(Value::Int(map_entries(&receiver).borrow().len() as i64))
}

fn map_is_empty(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    if !args.is_empty() {
        return Err(interpreter.runtime_error(span, "Map.isEmpty expects 0 arguments"));
    }
    Ok(Value::Bool(map_entries(&receiver).borrow().is_empty()))
}

fn map_non_empty(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    if !args.is_empty() {
        return Err(interpreter.runtime_error(span, "Map.nonEmpty expects 0 arguments"));
    }
    Ok(Value::Bool(!map_entries(&receiver).borrow().is_empty()))
}
