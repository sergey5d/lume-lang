use std::{cell::RefCell, rc::Rc};

use crate::{
    Diagnostic, Span,
    ast::TypeKind,
    interpreter::{
        Interpreter, Value, iterable_map_entries, iterable_values, map_put_entry, values_equal,
    },
    ir,
};

use crate::runtime::{
    RuntimeMethod, RuntimeMethodSlot, RuntimeMethodTarget, RuntimeType, RuntimeTypeId,
};

pub(super) fn define() -> RuntimeType {
    RuntimeType {
        id: RuntimeTypeId(usize::MAX),
        ir_type_id: None,
        kind: TypeKind::Class,
        name: "Map".to_string(),
        fields: Vec::new(),
        methods: vec![
            m(0, "++", vec![ir::Type::Unknown], map_concat),
            m(
                1,
                "put",
                vec![ir::Type::Unknown, ir::Type::Unknown],
                map_put,
            ),
            m(2, "iterator", Vec::new(), map_iterator),
            m(3, "map", vec![function_unknown()], map_map),
            m(4, "mapValues", vec![function_unknown()], map_map_values),
            m(5, "flatMap", vec![function_unknown()], map_flat_map),
            m(6, "filter", vec![function_unknown()], map_filter),
            m(
                7,
                "fold",
                vec![ir::Type::Unknown, function_unknown()],
                map_fold,
            ),
            m(8, "reduce", vec![function_unknown()], map_reduce),
            m(9, "exists", vec![function_unknown()], map_exists),
            m(10, "forAll", vec![function_unknown()], map_for_all),
            m(11, "forEach", vec![function_unknown()], map_for_each),
            m(12, "get", vec![ir::Type::Unknown], map_get),
            m(13, "contains", vec![ir::Type::Unknown], map_contains),
            m(14, "size", Vec::new(), map_size),
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

fn m(
    slot: usize,
    name: &str,
    params: Vec<ir::Type>,
    target: for<'a> fn(
        &mut Interpreter<'a>,
        Value,
        Vec<Value>,
        Option<Span>,
    ) -> Result<Value, Diagnostic>,
) -> RuntimeMethod {
    RuntimeMethod {
        slot: RuntimeMethodSlot(slot),
        name: name.to_string(),
        target: RuntimeMethodTarget::Builtin(target),
        params,
    }
}

fn map_entries(receiver: &Value) -> Rc<RefCell<Vec<(Value, Value)>>> {
    let Value::Map(entries) = receiver else {
        unreachable!();
    };
    entries.clone()
}

fn map_concat(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    let [other] = args.as_slice() else {
        return Err(interpreter.runtime_error(span, "operator ++ expects 1 argument"));
    };
    let entries = map_entries(&receiver);
    let mut next = entries.borrow().clone();
    for (key, value) in iterable_map_entries(other.clone(), span, interpreter)? {
        map_put_entry(&mut next, key, value);
    }
    Ok(Value::map(next))
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
    map_put_entry(
        &mut map_entries(&receiver).borrow_mut(),
        args[0].clone(),
        args[1].clone(),
    );
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
    let found = map_entries(&receiver)
        .borrow()
        .iter()
        .find(|(key, _)| values_equal(key, needle))
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
    Ok(Value::Bool(
        map_entries(&receiver)
            .borrow()
            .iter()
            .any(|(key, _)| values_equal(key, needle)),
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
