use crate::{
    Diagnostic, Span,
    ast::TypeKind,
    interpreter::{Interpreter, Value},
};
use regex::Regex;

use super::builtin_method;
use crate::runtime::{RuntimeType, RuntimeTypeId};

pub(super) fn define() -> RuntimeType {
    RuntimeType {
        id: RuntimeTypeId(usize::MAX),
        ir_type_id: None,
        kind: TypeKind::Class,
        name: "Str".to_string(),
        fields: Vec::new(),
        field_init: None,
        methods: vec![
            builtin_method(0, "size", Vec::new(), str_size),
            builtin_method(1, "split", vec![crate::ir::Type::Str], str_split),
            builtin_method(2, "runeAt", vec![crate::ir::Type::Int], str_rune_at),
            builtin_method(3, "compare", vec![crate::ir::Type::Str], str_compare),
            builtin_method(4, "trim", Vec::new(), str_trim),
            builtin_method(5, "isEmpty", Vec::new(), str_is_empty),
            builtin_method(6, "nonEmpty", Vec::new(), str_non_empty),
            builtin_method(7, "splitRegex", vec![crate::ir::Type::Str], str_split_regex),
        ],
        enum_cases: Vec::new(),
        with_bounds: Vec::new(),
    }
}

fn str_compare(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    let Value::String(left) = receiver else {
        unreachable!();
    };
    let [right] = args.as_slice() else {
        return Err(interpreter.runtime_error(span, "Str.compare expects 1 argument"));
    };
    let Value::String(right) = right else {
        return Err(interpreter.runtime_error(span, "Str.compare argument must be Str"));
    };
    Ok(Value::Int(match left.cmp(right) {
        std::cmp::Ordering::Less => -1,
        std::cmp::Ordering::Equal => 0,
        std::cmp::Ordering::Greater => 1,
    }))
}

fn string_rune_at(text: &str, index: i64) -> Option<char> {
    if index < 0 {
        return None;
    }
    text.chars().nth(index as usize)
}

fn str_size(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    let Value::String(text) = receiver else {
        unreachable!();
    };
    if !args.is_empty() {
        return Err(interpreter.runtime_error(span, "Str.size expects 0 arguments"));
    }
    Ok(Value::Int(text.chars().count() as i64))
}

fn str_is_empty(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    let Value::String(text) = receiver else {
        unreachable!();
    };
    if !args.is_empty() {
        return Err(interpreter.runtime_error(span, "Str.isEmpty expects 0 arguments"));
    }
    Ok(Value::Bool(text.is_empty()))
}

fn str_non_empty(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    let Value::String(text) = receiver else {
        unreachable!();
    };
    if !args.is_empty() {
        return Err(interpreter.runtime_error(span, "Str.nonEmpty expects 0 arguments"));
    }
    Ok(Value::Bool(!text.is_empty()))
}

fn str_split(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    let Value::String(text) = receiver else {
        unreachable!();
    };
    let [separator] = args.as_slice() else {
        return Err(interpreter.runtime_error(span, "Str.split expects 1 argument"));
    };
    let separator = match separator {
        Value::String(value) => value.clone(),
        _ => return Err(interpreter.runtime_error(span, "Str.split separator must be Str")),
    };
    Ok(Value::list(
        text.split(&separator)
            .map(|part| Value::String(part.to_string()))
            .collect(),
    ))
}

fn str_split_regex(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    let Value::String(text) = receiver else {
        unreachable!();
    };
    let [pattern] = args.as_slice() else {
        return Err(interpreter.runtime_error(span, "Str.splitRegex expects 1 argument"));
    };
    let pattern = match pattern {
        Value::String(value) => value.clone(),
        _ => return Err(interpreter.runtime_error(span, "Str.splitRegex pattern must be Str")),
    };
    let regex = Regex::new(&pattern).map_err(|err| {
        interpreter.runtime_error(
            span,
            format!("Str.splitRegex invalid regex '{}': {err}", pattern),
        )
    })?;
    Ok(Value::list(
        regex
            .split(&text)
            .map(|part| Value::String(part.to_string()))
            .collect(),
    ))
}

fn str_trim(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    let Value::String(text) = receiver else {
        unreachable!();
    };
    if !args.is_empty() {
        return Err(interpreter.runtime_error(span, "Str.trim expects 0 arguments"));
    }
    Ok(Value::String(text.trim().to_string()))
}

fn str_rune_at(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    let Value::String(text) = receiver else {
        unreachable!();
    };
    let [index] = args.as_slice() else {
        return Err(interpreter.runtime_error(span, "Str.runeAt expects 1 argument"));
    };
    let index = index.as_int(interpreter, span, "Str.runeAt index")?;
    Ok(match string_rune_at(&text, index) {
        Some(value) => interpreter.option_some(Value::Rune(value)),
        None => interpreter.option_none(),
    })
}
