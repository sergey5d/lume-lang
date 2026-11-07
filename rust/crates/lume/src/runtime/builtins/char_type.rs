use crate::{
    Diagnostic, Span,
    interpreter::{Interpreter, Value},
    ir,
    runtime::RuntimeType,
};

pub(super) fn define() -> RuntimeType {
    let mut ty = super::empty_builtin_type("Char");
    ty.methods = vec![super::builtin_method(
        0,
        "repeat",
        vec![ir::Type::Int],
        char_repeat,
    )];
    ty
}

fn char_repeat(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    let Value::Char(character) = receiver else {
        unreachable!();
    };
    super::str_type::repeat_text(
        interpreter,
        &character.to_string(),
        &args,
        span,
        "Char.repeat",
    )
}
