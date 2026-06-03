use crate::{ast::TypeKind, interpreter::{Interpreter, Value}, Diagnostic, Span};

use crate::runtime::{
    RuntimeMethod, RuntimeMethodSlot, RuntimeMethodTarget, RuntimeType, RuntimeTypeId,
};

pub(super) fn define() -> RuntimeType {
    RuntimeType {
        id: RuntimeTypeId(usize::MAX),
        ir_type_id: None,
        kind: TypeKind::Class,
        name: "Str".to_string(),
        fields: Vec::new(),
        methods: vec![
            RuntimeMethod {
                slot: RuntimeMethodSlot(0),
                name: "size".to_string(),
                target: RuntimeMethodTarget::Builtin(str_size),
                params: Vec::new(),
            },
            RuntimeMethod {
                slot: RuntimeMethodSlot(1),
                name: "split".to_string(),
                target: RuntimeMethodTarget::Builtin(str_split),
                params: vec![crate::ir::Type::Str],
            },
        ],
        enum_cases: Vec::new(),
        with_bounds: Vec::new(),
    }
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
