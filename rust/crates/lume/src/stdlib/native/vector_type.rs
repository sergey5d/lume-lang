use std::{cell::RefCell, rc::Rc};

use crate::{
    Diagnostic, Span,
    interpreter::{Interpreter, Value, iterable_values},
};

use super::builtin_method;
use crate::runtime::RuntimeType;

pub(super) fn define() -> RuntimeType {
    define_for("Vector")
}

pub(super) fn define_for(owner: &str) -> RuntimeType {
    let mut ty = super::empty_builtin_type(owner);
    ty.methods = vec![
        builtin_method(owner, "add", 1, list_add),
        builtin_method(owner, "addAll", 1, list_add_all),
        builtin_method(owner, "map", 1, list_map),
        builtin_method(owner, "flatMap", 1, list_flat_map),
        builtin_method(owner, "flatten", 0, list_flatten),
        builtin_method(owner, "filter", 1, list_filter),
        builtin_method(owner, "fold", 2, list_fold),
        builtin_method(owner, "reduce", 1, list_reduce),
        builtin_method(owner, "exists", 1, list_exists),
        builtin_method(owner, "forEach", 1, list_for_each),
        builtin_method(owner, "forAll", 1, list_for_all),
        builtin_method(owner, "sort", 1, list_sort),
        builtin_method(owner, "sort", 0, list_sort),
        builtin_method(owner, "take", 1, list_take),
        builtin_method(owner, "zip", 1, list_zip),
        builtin_method(owner, "zipWithIndex", 0, list_zip_with_index),
        builtin_method(owner, "size", 0, list_size),
        builtin_method(owner, "isEmpty", 0, list_is_empty),
        builtin_method(owner, "nonEmpty", 0, list_non_empty),
        builtin_method(owner, "makeStr", 1, list_make_str),
        builtin_method(owner, "makeStr", 2, list_make_str),
        builtin_method(owner, "at", 1, list_at),
        builtin_method(owner, "slice", 0, list_slice),
        builtin_method(owner, "slice", 1, list_slice),
        builtin_method(owner, "slice", 2, list_slice),
        builtin_method(owner, "setAt", 2, list_set_at),
        builtin_method(owner, "insertAt", 2, list_insert_at),
        builtin_method(owner, "removeAt", 1, list_remove_at),
        builtin_method(owner, "removeFirst", 0, list_remove_first),
        builtin_method(owner, "removeLast", 0, list_remove_last),
        builtin_method(owner, "tail", 0, list_tail),
        builtin_method(owner, "first", 0, list_first),
        builtin_method(owner, "last", 0, list_last),
        builtin_method(owner, "contains", 1, list_contains),
        builtin_method(owner, "indexOf", 1, list_index_of),
        builtin_method(owner, "iterator", 0, list_iterator),
        builtin_method(owner, "clone", 0, list_clone),
        builtin_method(owner, "count", 1, list_count),
        builtin_method(owner, "find", 1, list_find),
    ]
    .into_iter()
    .flatten()
    .collect();
    ty
}

fn list_items(receiver: &Value) -> Rc<RefCell<Vec<Value>>> {
    let Value::List(items) = receiver else {
        unreachable!();
    };
    items.clone()
}

fn list_values(
    interpreter: &Interpreter<'_>,
    receiver: &Value,
    span: Option<Span>,
    context: &str,
) -> Result<Vec<Value>, Diagnostic> {
    let _ = (span, context);
    let items = list_items(receiver);
    let items = items.borrow();
    Ok(interpreter.clone_values(&items))
}

fn values_match(
    interpreter: &mut Interpreter<'_>,
    left: &Value,
    right: &Value,
    span: Option<Span>,
    context: &str,
) -> Result<bool, Diagnostic> {
    interpreter.ensure_observable_value(left, span, context)?;
    interpreter.ensure_observable_value(right, span, context)?;
    interpreter.values_equal(left, right, span)
}

fn list_add(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    if args.len() != 1 {
        return Err(interpreter.runtime_error(span, "Vector.add expects 1 argument"));
    }
    let items = list_items(&receiver);
    interpreter.check_collection_size(items.borrow().len().saturating_add(1), span)?;
    items.borrow_mut().push(args[0].clone());
    Ok(receiver)
}

fn list_add_all(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    let [other] = args.as_slice() else {
        return Err(interpreter.runtime_error(span, "Vector.addAll expects 1 argument"));
    };
    let rhs = iterable_values(other.clone(), span, interpreter)?;
    interpreter.check_collection_size(
        list_items(&receiver)
            .borrow()
            .len()
            .saturating_add(rhs.len()),
        span,
    )?;
    interpreter.charge_work(rhs.len() as u64, span)?;
    list_items(&receiver).borrow_mut().extend(rhs);
    Ok(receiver)
}

fn list_map(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    let [callback] = args.as_slice() else {
        return Err(interpreter.runtime_error(span, "Vector.map expects 1 argument"));
    };
    let values = list_values(interpreter, &receiver, span, "Vector.map")?;
    let mut out = Vec::with_capacity(values.len());
    for value in values {
        out.push(interpreter.invoke_value(callback.clone(), vec![value], span)?);
    }
    Ok(Value::list(out))
}

fn list_flat_map(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    let [callback] = args.as_slice() else {
        return Err(interpreter.runtime_error(span, "Vector.flatMap expects 1 argument"));
    };
    let values = list_values(interpreter, &receiver, span, "Vector.flatMap")?;
    let mut out = Vec::new();
    for value in values {
        let mapped = interpreter.invoke_value(callback.clone(), vec![value], span)?;
        let next = iterable_values(mapped, span, interpreter)?;
        interpreter.check_collection_size(out.len().saturating_add(next.len()), span)?;
        interpreter.charge_work(next.len() as u64, span)?;
        out.extend(next);
    }
    Ok(Value::list(out))
}

fn list_flatten(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    if !args.is_empty() {
        return Err(interpreter.runtime_error(span, "Vector.flatten expects no arguments"));
    }
    let values = list_values(interpreter, &receiver, span, "Vector.flatten")?;
    let mut out = Vec::new();
    for value in values {
        let next = iterable_values(value, span, interpreter)?;
        interpreter.check_collection_size(out.len().saturating_add(next.len()), span)?;
        interpreter.charge_work(next.len() as u64, span)?;
        out.extend(next);
    }
    Ok(Value::list(out))
}

fn list_filter(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    let [callback] = args.as_slice() else {
        return Err(interpreter.runtime_error(span, "Vector.filter expects 1 argument"));
    };
    let values = list_values(interpreter, &receiver, span, "Vector.filter")?;
    let mut out = Vec::new();
    for value in values {
        if interpreter
            .invoke_value(callback.clone(), vec![value.clone()], span)?
            .as_bool(interpreter, span, "Vector.filter predicate")?
        {
            out.push(value);
        }
    }
    Ok(Value::list(out))
}

fn list_fold(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    if args.len() != 2 {
        return Err(interpreter.runtime_error(span, "Vector.fold expects 2 arguments"));
    }
    let mut acc = args[0].clone();
    let callback = args[1].clone();
    let values = list_values(interpreter, &receiver, span, "Vector.fold")?;
    for value in values {
        acc = interpreter.invoke_value(callback.clone(), vec![acc, value], span)?;
    }
    Ok(acc)
}

fn list_reduce(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    let [callback] = args.as_slice() else {
        return Err(interpreter.runtime_error(span, "Vector.reduce expects 1 argument"));
    };
    let values = list_values(interpreter, &receiver, span, "Vector.reduce")?;
    let Some((first, rest)) = values.split_first() else {
        return Ok(interpreter.option_none());
    };
    let mut acc = first.clone();
    for value in rest {
        acc = interpreter.invoke_value(callback.clone(), vec![acc, value.clone()], span)?;
    }
    Ok(interpreter.option_some(acc))
}

fn list_exists(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    let [callback] = args.as_slice() else {
        return Err(interpreter.runtime_error(span, "Vector.exists expects 1 argument"));
    };
    let values = list_values(interpreter, &receiver, span, "Vector.exists")?;
    for value in values {
        if interpreter
            .invoke_value(callback.clone(), vec![value], span)?
            .as_bool(interpreter, span, "Vector.exists predicate")?
        {
            return Ok(Value::Bool(true));
        }
    }
    Ok(Value::Bool(false))
}

fn list_for_each(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    let [callback] = args.as_slice() else {
        return Err(interpreter.runtime_error(span, "Vector.forEach expects 1 argument"));
    };
    let values = list_values(interpreter, &receiver, span, "Vector.forEach")?;
    for value in values {
        let _ = interpreter.invoke_value(callback.clone(), vec![value], span)?;
    }
    Ok(Value::Unit)
}

fn list_for_all(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    let [callback] = args.as_slice() else {
        return Err(interpreter.runtime_error(span, "Vector.forAll expects 1 argument"));
    };
    let values = list_values(interpreter, &receiver, span, "Vector.forAll")?;
    for value in values {
        if !interpreter
            .invoke_value(callback.clone(), vec![value], span)?
            .as_bool(interpreter, span, "Vector.forAll predicate")?
        {
            return Ok(Value::Bool(false));
        }
    }
    Ok(Value::Bool(true))
}

fn list_sort(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    let compare = match args.as_slice() {
        [] => None,
        [compare] => Some(compare.clone()),
        _ => {
            return Err(interpreter.runtime_error(
                span,
                "Vector.sort expects no arguments or one comparator function",
            ));
        }
    };
    let items = list_items(&receiver);
    let mut values = {
        let items = items.borrow();
        interpreter.clone_values(&items)
    };
    let len = values.len();
    for i in 0..len {
        for j in (i + 1)..len {
            let cmp = if let Some(compare) = &compare {
                interpreter.invoke_value(
                    compare.clone(),
                    vec![values[i].clone(), values[j].clone()],
                    span,
                )?
            } else {
                interpreter.invoke_method(
                    values[i].clone(),
                    "compareTo",
                    vec![values[j].clone()],
                    span,
                )?
            };
            if cmp.as_int(interpreter, span, "Vector.sort comparison result")? > 0 {
                values.swap(i, j);
            }
        }
    }
    *items.borrow_mut() = values;
    Ok(receiver)
}

fn list_take(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    let [count] = args.as_slice() else {
        return Err(interpreter.runtime_error(span, "Vector.take expects 1 argument"));
    };
    let count = count.as_int(interpreter, span, "Vector.take count")?;
    let values = list_values(interpreter, &receiver, span, "Vector.take")?;
    let end = usize::try_from(count).unwrap_or(0).min(values.len());
    Ok(Value::list(values[..end].to_vec()))
}

fn list_zip(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    let [other] = args.as_slice() else {
        return Err(interpreter.runtime_error(span, "Vector.zip expects 1 argument"));
    };
    let lhs = list_values(interpreter, &receiver, span, "Vector.zip")?;
    let rhs = iterable_values(other.clone(), span, interpreter)?;
    Ok(Value::list(
        lhs.into_iter()
            .zip(rhs)
            .map(|(left, right)| Value::Tuple(vec![left, right]))
            .collect(),
    ))
}

fn list_zip_with_index(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    if !args.is_empty() {
        return Err(interpreter.runtime_error(span, "Vector.zipWithIndex expects 0 arguments"));
    }
    let items = list_values(interpreter, &receiver, span, "Vector.zipWithIndex")?;
    Ok(Value::list(
        items
            .into_iter()
            .enumerate()
            .map(|(index, value)| Value::Tuple(vec![value, Value::Int(index as i64)]))
            .collect(),
    ))
}

fn list_size(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    if !args.is_empty() {
        return Err(interpreter.runtime_error(span, "Vector.size expects 0 arguments"));
    }
    Ok(Value::Int(list_items(&receiver).borrow().len() as i64))
}

fn list_is_empty(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    if !args.is_empty() {
        return Err(interpreter.runtime_error(span, "Vector.isEmpty expects 0 arguments"));
    }
    Ok(Value::Bool(list_items(&receiver).borrow().is_empty()))
}

fn list_non_empty(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    if !args.is_empty() {
        return Err(interpreter.runtime_error(span, "Vector.nonEmpty expects 0 arguments"));
    }
    Ok(Value::Bool(!list_items(&receiver).borrow().is_empty()))
}

fn list_make_str(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    let (separator, render) = match args.as_slice() {
        [separator] => (separator, None),
        [separator, render] => (separator, Some(render)),
        _ => {
            return Err(interpreter.runtime_error(span, "Vector.makeStr expects 1 or 2 arguments"));
        }
    };
    let Value::String(separator) = separator else {
        return Err(interpreter.runtime_error(
            span,
            format!(
                "Vector.makeStr expects Str separator, got {}",
                separator.render()
            ),
        ));
    };

    let items = list_values(interpreter, &receiver, span, "Vector.makeStr")?;
    let mut rendered = String::new();
    for (index, item) in items.iter().enumerate() {
        if index > 0 {
            rendered.push_str(separator);
        }
        let item = if let Some(render) = render {
            interpreter.invoke_value(render.clone(), vec![item.clone()], span)?
        } else {
            item.clone()
        };
        if render.is_some() {
            let Value::String(item) = item else {
                return Err(interpreter.runtime_error(
                    span,
                    format!(
                        "Vector.makeStr formatter must return Str, got {}",
                        item.render()
                    ),
                ));
            };
            rendered.push_str(&item);
        } else {
            rendered.push_str(&interpreter.render_value(&item, span, "Vector.makeStr")?);
        }
    }
    Ok(Value::String(rendered))
}

fn list_at(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    if args.len() != 1 {
        return Err(interpreter.runtime_error(span, "at expects 1 argument"));
    }
    let index = args[0].as_int(interpreter, span, "at index")?;
    let items = list_items(&receiver);
    let items = items.borrow();
    let value = usize::try_from(index)
        .ok()
        .and_then(|index| items.get(index))
        .map(|value| interpreter.clone_value(value));
    Ok(match value {
        Some(value) => interpreter.option_some(value),
        None => interpreter.option_none(),
    })
}

fn list_slice(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    if args.len() > 2 {
        return Err(interpreter.runtime_error(span, "Vector.slice expects 0, 1, or 2 arguments"));
    }
    let values = list_values(interpreter, &receiver, span, "Vector.slice")?;
    let start = match args.first() {
        Some(value) => value.as_int(interpreter, span, "Vector.slice start")?,
        None => 0,
    };
    let end = match args.get(1) {
        Some(value) => value.as_int(interpreter, span, "Vector.slice end")?,
        None => values.len() as i64,
    };
    let Ok(start_index) = usize::try_from(start) else {
        return Err(interpreter.runtime_error(
            span,
            format!("Vector.slice start {} is out of bounds", start),
        ));
    };
    let Ok(end_index) = usize::try_from(end) else {
        return Err(
            interpreter.runtime_error(span, format!("Vector.slice end {} is out of bounds", end))
        );
    };
    if start_index > end_index || end_index > values.len() {
        return Err(interpreter.runtime_error(
            span,
            format!(
                "Vector.slice range {}:{} is out of bounds for size {}",
                start,
                end,
                values.len()
            ),
        ));
    }
    Ok(Value::list(values[start_index..end_index].to_vec()))
}

fn list_set_at(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    if args.len() != 2 {
        return Err(interpreter.runtime_error(span, "setAt expects 2 arguments"));
    }
    let index = args[0].as_int(interpreter, span, "setAt index")?;
    let items = list_items(&receiver);
    let mut items = items.borrow_mut();
    if index < 0 || index as usize >= items.len() {
        return Ok(invalid_index_result(interpreter, index, items.len()));
    }
    let previous = std::mem::replace(&mut items[index as usize], args[1].clone());
    Ok(interpreter.result_ok(previous))
}

fn list_insert_at(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    if args.len() != 2 {
        return Err(interpreter.runtime_error(span, "insertAt expects 2 arguments"));
    }
    let index = args[0].as_int(interpreter, span, "insertAt index")?;
    let items = list_items(&receiver);
    let mut items = items.borrow_mut();
    if index < 0 || index as usize > items.len() {
        return Ok(invalid_index_result(interpreter, index, items.len()));
    }
    items.insert(index as usize, args[1].clone());
    Ok(interpreter.result_ok(Value::Unit))
}

fn list_remove_at(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    if args.len() != 1 {
        return Err(interpreter.runtime_error(span, "removeAt expects 1 argument"));
    }
    let index = args[0].as_int(interpreter, span, "removeAt index")?;
    let items = list_items(&receiver);
    let mut items = items.borrow_mut();
    if index < 0 || index as usize >= items.len() {
        return Ok(invalid_index_result(interpreter, index, items.len()));
    }
    Ok(interpreter.result_ok(items.remove(index as usize)))
}

fn invalid_index_result(interpreter: &Interpreter<'_>, index: i64, size: usize) -> Value {
    interpreter.result_err(Value::Record(Rc::new(RefCell::new(vec![
        ("index".to_string(), Value::Int(index)),
        ("size".to_string(), Value::Int(size as i64)),
    ]))))
}

fn list_remove_first(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    if !args.is_empty() {
        return Err(interpreter.runtime_error(span, "Vector.removeFirst expects 0 arguments"));
    }
    let items = list_items(&receiver);
    let mut items = items.borrow_mut();
    if items.is_empty() {
        return Ok(interpreter.option_none());
    }
    let value = items.remove(0);
    Ok(interpreter.option_some(value))
}

fn list_remove_last(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    if !args.is_empty() {
        return Err(interpreter.runtime_error(span, "Vector.removeLast expects 0 arguments"));
    }
    let value = list_items(&receiver).borrow_mut().pop();
    Ok(match value {
        Some(value) => interpreter.option_some(value),
        None => interpreter.option_none(),
    })
}

fn list_tail(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    if !args.is_empty() {
        return Err(interpreter.runtime_error(span, "Vector.tail expects 0 arguments"));
    }
    let items = list_items(&receiver);
    let values = items.borrow();
    let tail = if values.len() <= 1 {
        Vec::new()
    } else {
        interpreter.clone_values(&values[1..])
    };
    Ok(Value::list(tail))
}

fn list_first(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    if !args.is_empty() {
        return Err(interpreter.runtime_error(span, "Vector.first expects 0 arguments"));
    }
    let items = list_items(&receiver);
    let items = items.borrow();
    let value = items.first().map(|value| interpreter.clone_value(value));
    Ok(match value {
        Some(value) => interpreter.option_some(value),
        None => interpreter.option_none(),
    })
}

fn list_last(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    if !args.is_empty() {
        return Err(interpreter.runtime_error(span, "Vector.last expects 0 arguments"));
    }
    let items = list_items(&receiver);
    let items = items.borrow();
    let value = items.last().map(|value| interpreter.clone_value(value));
    Ok(match value {
        Some(value) => interpreter.option_some(value),
        None => interpreter.option_none(),
    })
}

pub(super) fn list_count(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    let [callback] = args.as_slice() else {
        return Err(interpreter.runtime_error(span, "Vector.count expects 1 argument"));
    };
    let values = list_values(interpreter, &receiver, span, "Vector.count")?;
    let mut count = 0i64;
    for value in values {
        if interpreter
            .invoke_value(callback.clone(), vec![value], span)?
            .as_bool(interpreter, span, "Vector.count predicate")?
        {
            count += 1;
        }
    }
    Ok(Value::Int(count))
}

fn list_contains(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    let [needle] = args.as_slice() else {
        return Err(interpreter.runtime_error(span, "Vector.contains expects 1 argument"));
    };
    let items = list_items(&receiver);
    for value in items.borrow().iter() {
        if values_match(interpreter, value, needle, span, "Vector.contains")? {
            return Ok(Value::Bool(true));
        }
    }
    Ok(Value::Bool(false))
}

pub(super) fn list_find(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    let [needle] = args.as_slice() else {
        return Err(interpreter.runtime_error(span, "Vector.find expects 1 argument"));
    };
    let items = list_items(&receiver);
    for value in items.borrow().iter() {
        if values_match(interpreter, value, needle, span, "Vector.find")? {
            return Ok(interpreter.option_some(value.clone()));
        }
    }
    Ok(interpreter.option_none())
}

fn list_index_of(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    let [needle] = args.as_slice() else {
        return Err(interpreter.runtime_error(span, "Vector.indexOf expects 1 argument"));
    };
    let items = list_items(&receiver);
    for (index, value) in items.borrow().iter().enumerate() {
        if values_match(interpreter, value, needle, span, "Vector.indexOf")? {
            return Ok(interpreter.option_some(Value::Int(index as i64)));
        }
    }
    Ok(interpreter.option_none())
}

fn list_iterator(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    if !args.is_empty() {
        return Err(interpreter.runtime_error(span, "Vector.iterator expects 0 arguments"));
    }
    Ok(Value::iterator_from_values(
        list_items(&receiver).borrow().clone(),
    ))
}

fn list_clone(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    if !args.is_empty() {
        return Err(interpreter.runtime_error(span, "Array.clone expects 0 arguments"));
    }
    Ok(Value::list(list_values(
        interpreter,
        &receiver,
        span,
        "Array.clone",
    )?))
}
