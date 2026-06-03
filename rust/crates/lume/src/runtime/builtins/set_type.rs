use std::{cell::RefCell, rc::Rc};

use crate::{
    ast::TypeKind,
    interpreter::{iterable_values, push_unique, values_equal, Interpreter, Value},
    ir,
    Diagnostic, Span,
};

use crate::runtime::{
    RuntimeMethod, RuntimeMethodSlot, RuntimeMethodTarget, RuntimeType, RuntimeTypeId,
};

pub(super) fn define() -> RuntimeType {
    RuntimeType {
        id: RuntimeTypeId(usize::MAX),
        ir_type_id: None,
        kind: TypeKind::Class,
        name: "Set".to_string(),
        fields: Vec::new(),
        methods: vec![
            m(0, ":+", vec![ir::Type::Unknown], set_plus),
            m(1, "++", vec![ir::Type::Unknown], set_concat),
            m(2, "add", vec![ir::Type::Unknown], set_add),
            m(3, "iterator", Vec::new(), set_iterator),
            m(4, "map", vec![function_unknown()], set_map),
            m(5, "flatMap", vec![function_unknown()], set_flat_map),
            m(6, "filter", vec![function_unknown()], set_filter),
            m(7, "fold", vec![ir::Type::Unknown, function_unknown()], set_fold),
            m(8, "reduce", vec![function_unknown()], set_reduce),
            m(9, "exists", vec![function_unknown()], set_exists),
            m(10, "forAll", vec![function_unknown()], set_for_all),
            m(11, "forEach", vec![function_unknown()], set_for_each),
            m(12, "contains", vec![ir::Type::Unknown], set_contains),
            m(13, "size", Vec::new(), set_size),
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

fn set_items(receiver: &Value) -> Rc<RefCell<Vec<Value>>> {
    let Value::Set(items) = receiver else {
        unreachable!();
    };
    items.clone()
}

fn set_plus(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    let [value] = args.as_slice() else {
        return Err(interpreter.runtime_error(span, "operator :+ expects 1 argument"));
    };
    let items = set_items(&receiver);
    let mut next = items.borrow().clone();
    push_unique(&mut next, value.clone());
    Ok(Value::set(next))
}

fn set_concat(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    let [other] = args.as_slice() else {
        return Err(interpreter.runtime_error(span, "operator ++ expects 1 argument"));
    };
    let items = set_items(&receiver);
    let rhs = iterable_values(other.clone(), span, interpreter)?;
    let mut next = items.borrow().clone();
    for value in rhs {
        push_unique(&mut next, value);
    }
    Ok(Value::set(next))
}

fn set_add(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    let [value] = args.as_slice() else {
        return Err(interpreter.runtime_error(span, "Set.add expects 1 argument"));
    };
    push_unique(&mut set_items(&receiver).borrow_mut(), value.clone());
    Ok(receiver)
}

fn set_iterator(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    if !args.is_empty() {
        return Err(interpreter.runtime_error(span, "Set.iterator expects 0 arguments"));
    }
    Ok(Value::iterator_from_values(set_items(&receiver).borrow().clone()))
}

fn set_map(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    let [callback] = args.as_slice() else {
        return Err(interpreter.runtime_error(span, "Set.map expects 1 argument"));
    };
    let values = set_items(&receiver).borrow().clone();
    let mut out = Vec::new();
    for value in values {
        let mapped = interpreter.invoke_value(callback.clone(), vec![value], span)?;
        push_unique(&mut out, mapped);
    }
    Ok(Value::set(out))
}

fn set_flat_map(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    let [callback] = args.as_slice() else {
        return Err(interpreter.runtime_error(span, "Set.flatMap expects 1 argument"));
    };
    let values = set_items(&receiver).borrow().clone();
    let mut out = Vec::new();
    for value in values {
        let mapped = interpreter.invoke_value(callback.clone(), vec![value], span)?;
        for item in iterable_values(mapped, span, interpreter)? {
            push_unique(&mut out, item);
        }
    }
    Ok(Value::set(out))
}

fn set_filter(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    let [callback] = args.as_slice() else {
        return Err(interpreter.runtime_error(span, "Set.filter expects 1 argument"));
    };
    let values = set_items(&receiver).borrow().clone();
    let mut out = Vec::new();
    for value in values {
        if interpreter
            .invoke_value(callback.clone(), vec![value.clone()], span)?
            .as_bool(interpreter, span, "Set.filter predicate")?
        {
            push_unique(&mut out, value);
        }
    }
    Ok(Value::set(out))
}

fn set_fold(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    if args.len() != 2 {
        return Err(interpreter.runtime_error(span, "Set.fold expects 2 arguments"));
    }
    let mut acc = args[0].clone();
    let callback = args[1].clone();
    let values = set_items(&receiver).borrow().clone();
    for value in values {
        acc = interpreter.invoke_value(callback.clone(), vec![acc, value], span)?;
    }
    Ok(acc)
}

fn set_reduce(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    let [callback] = args.as_slice() else {
        return Err(interpreter.runtime_error(span, "Set.reduce expects 1 argument"));
    };
    let values = set_items(&receiver).borrow().clone();
    let Some((first, rest)) = values.split_first() else {
        return Ok(Value::option_none());
    };
    let mut acc = first.clone();
    for value in rest {
        acc = interpreter.invoke_value(callback.clone(), vec![acc, value.clone()], span)?;
    }
    Ok(Value::option_some(acc))
}

fn set_exists(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    let [callback] = args.as_slice() else {
        return Err(interpreter.runtime_error(span, "Set.exists expects 1 argument"));
    };
    let values = set_items(&receiver).borrow().clone();
    for value in values {
        if interpreter
            .invoke_value(callback.clone(), vec![value], span)?
            .as_bool(interpreter, span, "Set.exists predicate")?
        {
            return Ok(Value::Bool(true));
        }
    }
    Ok(Value::Bool(false))
}

fn set_for_all(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    let [callback] = args.as_slice() else {
        return Err(interpreter.runtime_error(span, "Set.forAll expects 1 argument"));
    };
    let values = set_items(&receiver).borrow().clone();
    for value in values {
        if !interpreter
            .invoke_value(callback.clone(), vec![value], span)?
            .as_bool(interpreter, span, "Set.forAll predicate")?
        {
            return Ok(Value::Bool(false));
        }
    }
    Ok(Value::Bool(true))
}

fn set_for_each(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    let [callback] = args.as_slice() else {
        return Err(interpreter.runtime_error(span, "Set.forEach expects 1 argument"));
    };
    let values = set_items(&receiver).borrow().clone();
    for value in values {
        let _ = interpreter.invoke_value(callback.clone(), vec![value], span)?;
    }
    Ok(Value::Unit)
}

fn set_contains(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    let [needle] = args.as_slice() else {
        return Err(interpreter.runtime_error(span, "Set.contains expects 1 argument"));
    };
    Ok(Value::Bool(
        set_items(&receiver)
            .borrow()
            .iter()
            .any(|value| values_equal(value, needle)),
    ))
}

fn set_size(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    if !args.is_empty() {
        return Err(interpreter.runtime_error(span, "Set.size expects 0 arguments"));
    }
    Ok(Value::Int(set_items(&receiver).borrow().len() as i64))
}
