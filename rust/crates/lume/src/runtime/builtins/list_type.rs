use std::{cell::RefCell, rc::Rc};

use crate::{
    Diagnostic, Span,
    ast::TypeKind,
    interpreter::{Interpreter, Value, iterable_values, values_equal},
    ir,
};

use super::builtin_method;
use crate::runtime::{RuntimeType, RuntimeTypeId};

pub(super) fn define() -> RuntimeType {
    RuntimeType {
        id: RuntimeTypeId(usize::MAX),
        ir_type_id: None,
        kind: TypeKind::Class,
        name: "List".to_string(),
        fields: Vec::new(),
        methods: vec![
            builtin_method(0, ":+", vec![ir::Type::Unknown], list_append_copy),
            builtin_method(1, "++", vec![ir::Type::Unknown], list_concat),
            builtin_method(2, "append", vec![ir::Type::Unknown], list_append_mut),
            builtin_method(29, "add", vec![ir::Type::Unknown], list_append_mut),
            builtin_method(30, "addAll", vec![ir::Type::Unknown], list_add_all),
            builtin_method(3, "map", vec![function_unknown()], list_map),
            builtin_method(4, "flatMap", vec![function_unknown()], list_flat_map),
            builtin_method(5, "filter", vec![function_unknown()], list_filter),
            builtin_method(
                6,
                "fold",
                vec![ir::Type::Unknown, function_unknown()],
                list_fold,
            ),
            builtin_method(7, "reduce", vec![function_unknown()], list_reduce),
            builtin_method(8, "exists", vec![function_unknown()], list_exists),
            builtin_method(9, "forEach", vec![function_unknown()], list_for_each),
            builtin_method(10, "forAll", vec![function_unknown()], list_for_all),
            builtin_method(11, "sort", vec![ir::Type::Unknown], list_sort),
            builtin_method(12, "zip", vec![ir::Type::Unknown], list_zip),
            builtin_method(13, "zipWithIndex", Vec::new(), list_zip_with_index),
            builtin_method(14, "size", Vec::new(), list_size),
            builtin_method(15, "isEmpty", Vec::new(), list_is_empty),
            builtin_method(16, "get", vec![ir::Type::Int], list_get),
            builtin_method(17, "remove", vec![ir::Type::Int], list_remove),
            builtin_method(18, "removeLast", Vec::new(), list_remove_last),
            builtin_method(19, "head", Vec::new(), list_head),
            builtin_method(20, "tail", Vec::new(), list_tail),
            builtin_method(21, "first", Vec::new(), list_first),
            builtin_method(22, "last", Vec::new(), list_last),
            builtin_method(24, "count", vec![function_unknown()], list_count),
            builtin_method(25, "contains", vec![ir::Type::Unknown], list_contains),
            builtin_method(26, "find", vec![ir::Type::Unknown], list_find),
            builtin_method(27, "indexOf", vec![ir::Type::Unknown], list_index_of),
            builtin_method(28, "iterator", Vec::new(), list_iterator),
        ],
        enum_cases: Vec::new(),
        with_bounds: Vec::new(),
    }
}

fn function_unknown() -> ir::Type {
    ir::Type::Function {
        params: Vec::new(),
        ret: Box::new(ir::Type::Unknown),
    }
}

fn list_items(receiver: &Value) -> Rc<RefCell<Vec<Value>>> {
    let Value::List(items) = receiver else {
        unreachable!();
    };
    items.clone()
}

fn list_append_copy(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    let [value] = args.as_slice() else {
        return Err(interpreter.runtime_error(span, "operator :+ expects 1 argument"));
    };
    let items = list_items(&receiver);
    let mut next = items.borrow().clone();
    next.push(value.clone());
    Ok(Value::list(next))
}

fn list_concat(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    let [other] = args.as_slice() else {
        return Err(interpreter.runtime_error(span, "operator ++ expects 1 argument"));
    };
    let items = list_items(&receiver);
    let rhs = iterable_values(other.clone(), span, interpreter)?;
    let mut next = items.borrow().clone();
    next.extend(rhs);
    Ok(Value::list(next))
}

fn list_append_mut(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    if args.len() != 1 {
        return Err(interpreter.runtime_error(span, "List.append expects 1 argument"));
    }
    list_items(&receiver).borrow_mut().push(args[0].clone());
    Ok(receiver)
}

fn list_add_all(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    let [other] = args.as_slice() else {
        return Err(interpreter.runtime_error(span, "List.addAll expects 1 argument"));
    };
    let rhs = iterable_values(other.clone(), span, interpreter)?;
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
        return Err(interpreter.runtime_error(span, "List.map expects 1 argument"));
    };
    let values = list_items(&receiver).borrow().clone();
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
        return Err(interpreter.runtime_error(span, "List.flatMap expects 1 argument"));
    };
    let values = list_items(&receiver).borrow().clone();
    let mut out = Vec::new();
    for value in values {
        let mapped = interpreter.invoke_value(callback.clone(), vec![value], span)?;
        out.extend(iterable_values(mapped, span, interpreter)?);
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
        return Err(interpreter.runtime_error(span, "List.filter expects 1 argument"));
    };
    let values = list_items(&receiver).borrow().clone();
    let mut out = Vec::new();
    for value in values {
        if interpreter
            .invoke_value(callback.clone(), vec![value.clone()], span)?
            .as_bool(interpreter, span, "List.filter predicate")?
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
        return Err(interpreter.runtime_error(span, "List.fold expects 2 arguments"));
    }
    let mut acc = args[0].clone();
    let callback = args[1].clone();
    let values = list_items(&receiver).borrow().clone();
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
        return Err(interpreter.runtime_error(span, "List.reduce expects 1 argument"));
    };
    let values = list_items(&receiver).borrow().clone();
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
        return Err(interpreter.runtime_error(span, "List.exists expects 1 argument"));
    };
    let values = list_items(&receiver).borrow().clone();
    for value in values {
        if interpreter
            .invoke_value(callback.clone(), vec![value], span)?
            .as_bool(interpreter, span, "List.exists predicate")?
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
        return Err(interpreter.runtime_error(span, "List.forEach expects 1 argument"));
    };
    let values = list_items(&receiver).borrow().clone();
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
        return Err(interpreter.runtime_error(span, "List.forAll expects 1 argument"));
    };
    let values = list_items(&receiver).borrow().clone();
    for value in values {
        if !interpreter
            .invoke_value(callback.clone(), vec![value], span)?
            .as_bool(interpreter, span, "List.forAll predicate")?
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
    let [ordering] = args.as_slice() else {
        return Err(interpreter.runtime_error(span, "List.sort expects 1 argument"));
    };
    let items = list_items(&receiver);
    let mut values = items.borrow().clone();
    let len = values.len();
    for i in 0..len {
        for j in (i + 1)..len {
            let cmp = interpreter.invoke_method(
                ordering.clone(),
                "compare",
                vec![values[i].clone(), values[j].clone()],
                span,
            )?;
            if cmp.as_int(interpreter, span, "Ordering.compare result")? > 0 {
                values.swap(i, j);
            }
        }
    }
    *items.borrow_mut() = values;
    Ok(receiver)
}

fn list_zip(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    let [other] = args.as_slice() else {
        return Err(interpreter.runtime_error(span, "List.zip expects 1 argument"));
    };
    let lhs = list_items(&receiver).borrow().clone();
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
        return Err(interpreter.runtime_error(span, "List.zipWithIndex expects 0 arguments"));
    }
    let items = list_items(&receiver);
    Ok(Value::list(
        items
            .borrow()
            .iter()
            .cloned()
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
        return Err(interpreter.runtime_error(span, "List.size expects 0 arguments"));
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
        return Err(interpreter.runtime_error(span, "List.isEmpty expects 0 arguments"));
    }
    Ok(Value::Bool(list_items(&receiver).borrow().is_empty()))
}

fn list_get(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    if args.len() != 1 {
        return Err(interpreter.runtime_error(span, "List.get expects 1 argument"));
    }
    let index = args[0].as_int(interpreter, span, "List.get index")?;
    let value = list_items(&receiver).borrow().get(index as usize).cloned();
    Ok(match value {
        Some(value) => interpreter.option_some(value),
        None => interpreter.option_none(),
    })
}

fn list_remove(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    if args.len() != 1 {
        return Err(interpreter.runtime_error(span, "List.remove expects 1 argument"));
    }
    let index = args[0].as_int(interpreter, span, "List.remove index")?;
    let items = list_items(&receiver);
    let mut items = items.borrow_mut();
    if index < 0 || index as usize >= items.len() {
        return Ok(interpreter.option_none());
    }
    Ok(interpreter.option_some(items.remove(index as usize)))
}

fn list_remove_last(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    if !args.is_empty() {
        return Err(interpreter.runtime_error(span, "List.removeLast expects 0 arguments"));
    }
    let value = list_items(&receiver).borrow_mut().pop();
    Ok(match value {
        Some(value) => interpreter.option_some(value),
        None => interpreter.option_none(),
    })
}

fn list_head(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    if !args.is_empty() {
        return Err(interpreter.runtime_error(span, "List.head expects 0 arguments"));
    }
    let value = list_items(&receiver).borrow().first().cloned();
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
        return Err(interpreter.runtime_error(span, "List.tail expects 0 arguments"));
    }
    let items = list_items(&receiver);
    let values = items.borrow();
    let tail = if values.len() <= 1 {
        Vec::new()
    } else {
        values[1..].to_vec()
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
        return Err(interpreter.runtime_error(span, "Array.first expects 0 arguments"));
    }
    let value = list_items(&receiver).borrow().first().cloned();
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
        return Err(interpreter.runtime_error(span, "Array.last expects 0 arguments"));
    }
    let value = list_items(&receiver).borrow().last().cloned();
    Ok(match value {
        Some(value) => interpreter.option_some(value),
        None => interpreter.option_none(),
    })
}

fn list_count(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    let [callback] = args.as_slice() else {
        return Err(interpreter.runtime_error(span, "Array.count expects 1 argument"));
    };
    let values = list_items(&receiver).borrow().clone();
    let mut count = 0i64;
    for value in values {
        if interpreter
            .invoke_value(callback.clone(), vec![value], span)?
            .as_bool(interpreter, span, "Array.count predicate")?
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
        return Err(interpreter.runtime_error(span, "Array.contains expects 1 argument"));
    };
    Ok(Value::Bool(
        list_items(&receiver)
            .borrow()
            .iter()
            .any(|value| values_equal(value, needle)),
    ))
}

fn list_find(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    let [needle] = args.as_slice() else {
        return Err(interpreter.runtime_error(span, "Array.find expects 1 argument"));
    };
    let value = list_items(&receiver)
        .borrow()
        .iter()
        .find(|value| values_equal(value, needle))
        .cloned();
    Ok(match value {
        Some(value) => interpreter.option_some(value),
        None => interpreter.option_none(),
    })
}

fn list_index_of(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    let [needle] = args.as_slice() else {
        return Err(interpreter.runtime_error(span, "Array.indexOf expects 1 argument"));
    };
    let index = list_items(&receiver)
        .borrow()
        .iter()
        .position(|value| values_equal(value, needle))
        .map(|index| index as i64)
        .unwrap_or(-1);
    Ok(Value::Int(index))
}

fn list_iterator(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    if !args.is_empty() {
        return Err(interpreter.runtime_error(span, "List.iterator expects 0 arguments"));
    }
    Ok(Value::iterator_from_values(
        list_items(&receiver).borrow().clone(),
    ))
}
