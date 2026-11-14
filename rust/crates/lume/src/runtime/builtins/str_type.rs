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
        mixin_init: None,
        methods: vec![
            builtin_method(0, "size", Vec::new(), str_size),
            builtin_method(1, "split", vec![crate::ir::Type::Str], str_split),
            builtin_method(2, "charAt", vec![crate::ir::Type::Int], str_char_at),
            builtin_method(3, "compare", vec![crate::ir::Type::Str], str_compare),
            builtin_method(4, "trim", Vec::new(), str_trim),
            builtin_method(5, "isEmpty", Vec::new(), str_is_empty),
            builtin_method(6, "nonEmpty", Vec::new(), str_non_empty),
            builtin_method(7, "splitRegex", vec![crate::ir::Type::Str], str_split_regex),
            builtin_method(8, "trimLeft", Vec::new(), str_trim_left),
            builtin_method(9, "trimRight", Vec::new(), str_trim_right),
            builtin_method(10, "toLower", Vec::new(), str_to_lower),
            builtin_method(11, "toUpper", Vec::new(), str_to_upper),
            builtin_method(12, "contains", vec![crate::ir::Type::Str], str_contains),
            builtin_method(13, "indexOf", vec![crate::ir::Type::Str], str_index_of),
            builtin_method(
                14,
                "replaceFirstRegex",
                vec![crate::ir::Type::Str, crate::ir::Type::Str],
                str_replace_first_regex,
            ),
            builtin_method(
                15,
                "replaceAllRegex",
                vec![crate::ir::Type::Str, crate::ir::Type::Str],
                str_replace_all_regex,
            ),
            builtin_method(16, "slice", Vec::new(), str_slice),
            builtin_method(17, "slice", vec![crate::ir::Type::Int], str_slice),
            builtin_method(
                18,
                "slice",
                vec![crate::ir::Type::Int, crate::ir::Type::Int],
                str_slice,
            ),
            builtin_method(
                19,
                "startsWith",
                vec![crate::ir::Type::Str],
                str_starts_with,
            ),
            builtin_method(20, "endsWith", vec![crate::ir::Type::Str], str_ends_with),
            builtin_method(
                21,
                "replaceFirst",
                vec![crate::ir::Type::Str, crate::ir::Type::Str],
                str_replace_first,
            ),
            builtin_method(
                22,
                "replaceAll",
                vec![crate::ir::Type::Str, crate::ir::Type::Str],
                str_replace_all,
            ),
            builtin_method(23, "isBlank", Vec::new(), str_is_blank),
            builtin_method(24, "lines", Vec::new(), str_lines),
            builtin_method(25, "repeat", vec![crate::ir::Type::Int], str_repeat),
            builtin_method(
                26,
                "lastIndexOf",
                vec![crate::ir::Type::Str],
                str_last_index_of,
            ),
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

fn string_char_at(text: &str, index: i64) -> Option<char> {
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

fn str_slice(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    let Value::String(text) = receiver else {
        unreachable!();
    };
    let size = text.chars().count() as i64;
    let (start, end) = match args.as_slice() {
        [] => (0, size),
        [start] => (start.as_int(interpreter, span, "Str.slice start")?, size),
        [start, end] => (
            start.as_int(interpreter, span, "Str.slice start")?,
            end.as_int(interpreter, span, "Str.slice end")?,
        ),
        _ => return Err(interpreter.runtime_error(span, "Str.slice expects 0, 1, or 2 arguments")),
    };
    if start < 0 || end < start || end > size {
        return Err(interpreter.runtime_error(
            span,
            format!("Str.slice range {start}:{end} is out of bounds for size {size}"),
        ));
    }
    Ok(Value::String(
        text.chars()
            .skip(start as usize)
            .take((end - start) as usize)
            .collect(),
    ))
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

fn str_is_blank(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    let Value::String(text) = receiver else {
        unreachable!()
    };
    if !args.is_empty() {
        return Err(interpreter.runtime_error(span, "Str.isBlank expects 0 arguments"));
    }
    Ok(Value::Bool(text.chars().all(char::is_whitespace)))
}

fn str_lines(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    let Value::String(text) = receiver else {
        unreachable!()
    };
    if !args.is_empty() {
        return Err(interpreter.runtime_error(span, "Str.lines expects 0 arguments"));
    }
    let mut lines = Vec::new();
    let bytes = text.as_bytes();
    let mut start = 0;
    let mut index = 0;
    while index < bytes.len() {
        if matches!(bytes[index], b'\n' | b'\r') {
            lines.push(Value::String(text[start..index].to_string()));
            if bytes[index] == b'\r' && bytes.get(index + 1) == Some(&b'\n') {
                index += 1;
            }
            start = index + 1;
        }
        index += 1;
    }
    if start < text.len() {
        lines.push(Value::String(text[start..].to_string()));
    }
    Ok(Value::list(lines))
}

fn str_repeat(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    let Value::String(text) = receiver else {
        unreachable!()
    };
    repeat_text(interpreter, &text, &args, span, "Str.repeat")
}

pub(super) fn repeat_text(
    interpreter: &mut Interpreter<'_>,
    text: &str,
    args: &[Value],
    span: Option<Span>,
    method: &str,
) -> Result<Value, Diagnostic> {
    let [count] = args else {
        return Err(interpreter.runtime_error(span, format!("{method} expects 1 argument")));
    };
    let count = count.as_int(interpreter, span, method)?;
    if count < 0 {
        return Err(interpreter.runtime_error(span, format!("{method} count must be non-negative")));
    }
    if count == 0 || text.is_empty() {
        return Ok(Value::String(String::new()));
    }
    let too_large = || interpreter.runtime_error(span, "repeated string is too large");
    let count = usize::try_from(count).map_err(|_| too_large())?;
    let size = text.len().checked_mul(count).ok_or_else(too_large)?;
    let mut result = String::new();
    result.try_reserve_exact(size).map_err(|_| too_large())?;
    for _ in 0..count {
        result.push_str(text);
    }
    Ok(Value::String(result))
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

fn str_trim_left(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    let Value::String(text) = receiver else {
        unreachable!();
    };
    if !args.is_empty() {
        return Err(interpreter.runtime_error(span, "Str.trimLeft expects 0 arguments"));
    }
    Ok(Value::String(text.trim_start().to_string()))
}

fn str_trim_right(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    let Value::String(text) = receiver else {
        unreachable!();
    };
    if !args.is_empty() {
        return Err(interpreter.runtime_error(span, "Str.trimRight expects 0 arguments"));
    }
    Ok(Value::String(text.trim_end().to_string()))
}

fn str_to_lower(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    let Value::String(text) = receiver else {
        unreachable!();
    };
    if !args.is_empty() {
        return Err(interpreter.runtime_error(span, "Str.toLower expects 0 arguments"));
    }
    Ok(Value::String(text.to_lowercase()))
}

fn str_to_upper(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    let Value::String(text) = receiver else {
        unreachable!();
    };
    if !args.is_empty() {
        return Err(interpreter.runtime_error(span, "Str.toUpper expects 0 arguments"));
    }
    Ok(Value::String(text.to_uppercase()))
}

fn str_contains(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    let Value::String(text) = receiver else {
        unreachable!();
    };
    let [part] = args.as_slice() else {
        return Err(interpreter.runtime_error(span, "Str.contains expects 1 argument"));
    };
    let Value::String(part) = part else {
        return Err(interpreter.runtime_error(span, "Str.contains argument must be Str"));
    };
    Ok(Value::Bool(text.contains(part)))
}

fn str_starts_with(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    let Value::String(text) = receiver else {
        unreachable!();
    };
    let [prefix] = args.as_slice() else {
        return Err(interpreter.runtime_error(span, "Str.startsWith expects 1 argument"));
    };
    let Value::String(prefix) = prefix else {
        return Err(interpreter.runtime_error(span, "Str.startsWith argument must be Str"));
    };
    Ok(Value::Bool(text.starts_with(prefix)))
}

fn str_ends_with(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    let Value::String(text) = receiver else {
        unreachable!();
    };
    let [suffix] = args.as_slice() else {
        return Err(interpreter.runtime_error(span, "Str.endsWith expects 1 argument"));
    };
    let Value::String(suffix) = suffix else {
        return Err(interpreter.runtime_error(span, "Str.endsWith argument must be Str"));
    };
    Ok(Value::Bool(text.ends_with(suffix)))
}

fn str_index_of(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    let Value::String(text) = receiver else {
        unreachable!();
    };
    let [part] = args.as_slice() else {
        return Err(interpreter.runtime_error(span, "Str.indexOf expects 1 argument"));
    };
    let Value::String(part) = part else {
        return Err(interpreter.runtime_error(span, "Str.indexOf argument must be Str"));
    };
    let Some(byte_index) = text.find(part) else {
        return Ok(interpreter.option_none());
    };
    let index = text[..byte_index].chars().count() as i64;
    Ok(interpreter.option_some(Value::Int(index)))
}

fn str_last_index_of(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    let Value::String(text) = receiver else {
        unreachable!()
    };
    let [part] = args.as_slice() else {
        return Err(interpreter.runtime_error(span, "Str.lastIndexOf expects 1 argument"));
    };
    let Value::String(part) = part else {
        return Err(interpreter.runtime_error(span, "Str.lastIndexOf argument must be Str"));
    };
    Ok(match text.rfind(part) {
        Some(index) => interpreter.option_some(Value::Int(text[..index].chars().count() as i64)),
        None => interpreter.option_none(),
    })
}

fn str_replace_first(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    str_replace_literal(interpreter, receiver, args, span, false)
}

fn str_replace_all(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    str_replace_literal(interpreter, receiver, args, span, true)
}

fn str_replace_literal(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
    replace_all: bool,
) -> Result<Value, Diagnostic> {
    let Value::String(text) = receiver else {
        unreachable!();
    };
    let [target, replacement] = args.as_slice() else {
        let method = if replace_all {
            "replaceAll"
        } else {
            "replaceFirst"
        };
        return Err(interpreter.runtime_error(span, format!("Str.{method} expects 2 arguments")));
    };
    let Value::String(target) = target else {
        return Err(interpreter.runtime_error(span, "Str replacement target must be Str"));
    };
    let Value::String(replacement) = replacement else {
        return Err(interpreter.runtime_error(span, "Str replacement value must be Str"));
    };
    let result = if replace_all {
        text.replace(target, replacement)
    } else {
        text.replacen(target, replacement, 1)
    };
    Ok(Value::String(result))
}

fn str_replace_first_regex(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    str_replace_regex(interpreter, receiver, args, span, false)
}

fn str_replace_all_regex(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    str_replace_regex(interpreter, receiver, args, span, true)
}

fn str_replace_regex(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
    replace_all: bool,
) -> Result<Value, Diagnostic> {
    let Value::String(text) = receiver else {
        unreachable!();
    };
    let [pattern, replacement] = args.as_slice() else {
        let method = if replace_all {
            "replaceAllRegex"
        } else {
            "replaceFirstRegex"
        };
        return Err(interpreter.runtime_error(span, format!("Str.{method} expects 2 arguments")));
    };
    let Value::String(pattern) = pattern else {
        return Err(interpreter.runtime_error(span, "Str regex pattern must be Str"));
    };
    let Value::String(replacement) = replacement else {
        return Err(interpreter.runtime_error(span, "Str regex replacement must be Str"));
    };
    let regex = Regex::new(pattern).map_err(|err| {
        interpreter.runtime_error(span, format!("invalid Str regex '{}': {err}", pattern))
    })?;
    let replaced = if replace_all {
        regex.replace_all(&text, replacement.as_str())
    } else {
        regex.replace(&text, replacement.as_str())
    };
    Ok(Value::String(replaced.into_owned()))
}

fn str_char_at(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    let Value::String(text) = receiver else {
        unreachable!();
    };
    let [index] = args.as_slice() else {
        return Err(interpreter.runtime_error(span, "Str.charAt expects 1 argument"));
    };
    let index = index.as_int(interpreter, span, "Str.charAt index")?;
    Ok(match string_char_at(&text, index) {
        Some(value) => interpreter.option_some(Value::Char(value)),
        None => interpreter.option_none(),
    })
}
