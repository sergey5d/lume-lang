use crate::{
    Diagnostic, Span,
    ast::TypeKind,
    interpreter::{Interpreter, Value},
};

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
            builtin_method(
                3,
                "expectRuneAt",
                vec![crate::ir::Type::Int],
                str_expect_rune_at,
            ),
        ],
        enum_cases: Vec::new(),
        with_bounds: Vec::new(),
    }
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

fn str_expect_rune_at(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    let Value::String(text) = receiver else {
        unreachable!();
    };
    let [index] = args.as_slice() else {
        return Err(interpreter.runtime_error(span, "Str.expectRuneAt expects 1 argument"));
    };
    let index = index.as_int(interpreter, span, "Str.expectRuneAt index")?;
    match string_rune_at(&text, index) {
        Some(value) => Ok(Value::Rune(value)),
        None => Err(interpreter.runtime_error(
            span,
            format!("Str.expectRuneAt index {} out of bounds", index),
        )),
    }
}
