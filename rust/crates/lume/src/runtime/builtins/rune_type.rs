use crate::{
    Diagnostic, Span,
    interpreter::{Interpreter, Value},
    ir,
    runtime::RuntimeType,
};

pub(super) fn define() -> RuntimeType {
    let mut ty = super::empty_builtin_type("Rune");
    ty.methods = vec![super::builtin_method(
        0,
        "repeat",
        vec![ir::Type::Int],
        rune_repeat,
    )];
    ty
}

fn rune_repeat(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    let Value::Rune(rune) = receiver else {
        unreachable!();
    };
    super::str_type::repeat_text(interpreter, &rune.to_string(), &args, span, "Rune.repeat")
}
