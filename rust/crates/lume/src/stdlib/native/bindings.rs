use super::*;

fn native_to_str(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    let rendered = receiver.render();
    interpreter
        .native_to_str(receiver, &args, span)?
        .ok_or_else(|| {
            interpreter.runtime_error(
                span,
                format!("method 'toStr' is not available on {rendered}"),
            )
        })
}

fn native_equals(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    let rendered = receiver.render();
    interpreter
        .native_equals(receiver, &args, span)?
        .ok_or_else(|| {
            interpreter.runtime_error(
                span,
                format!("method 'equals' is not available on {rendered}"),
            )
        })
}

fn native_hash(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    let rendered = receiver.render();
    interpreter
        .native_hash(receiver, &args, span)?
        .ok_or_else(|| {
            interpreter.runtime_error(
                span,
                format!("method 'hash' is not available on {rendered}"),
            )
        })
}

fn native_compare_to(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    let rendered = receiver.render();
    interpreter
        .native_compare_to(receiver, &args, span)?
        .ok_or_else(|| {
            interpreter.runtime_error(
                span,
                format!("method 'compareTo' is not available on {rendered}"),
            )
        })
}

fn native_sqrt(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    let rendered = receiver.render();
    interpreter
        .native_sqrt(receiver, &args, span)?
        .ok_or_else(|| {
            interpreter.runtime_error(
                span,
                format!("method 'sqrt' is not available on {rendered}"),
            )
        })
}

fn native_log(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    let rendered = receiver.render();
    interpreter
        .native_log(receiver, &args, span)?
        .ok_or_else(|| {
            interpreter.runtime_error(span, format!("method 'log' is not available on {rendered}"))
        })
}

fn native_abs(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    let rendered = receiver.render();
    interpreter
        .native_abs(receiver, &args, span)?
        .ok_or_else(|| {
            interpreter.runtime_error(span, format!("method 'abs' is not available on {rendered}"))
        })
}

fn native_pow(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    let rendered = receiver.render();
    interpreter
        .native_pow(receiver, &args, span)?
        .ok_or_else(|| {
            interpreter.runtime_error(span, format!("method 'pow' is not available on {rendered}"))
        })
}

fn native_wrapping_neg(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    let rendered = receiver.render();
    interpreter
        .native_wrapping_neg(receiver, &args, span)?
        .ok_or_else(|| {
            interpreter.runtime_error(
                span,
                format!("method 'wrappingNeg' is not available on {rendered}"),
            )
        })
}

fn native_wrapping_add(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    let rendered = receiver.render();
    interpreter
        .native_wrapping_add(receiver, &args, span)?
        .ok_or_else(|| {
            interpreter.runtime_error(
                span,
                format!("method 'wrappingAdd' is not available on {rendered}"),
            )
        })
}

fn native_wrapping_sub(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    let rendered = receiver.render();
    interpreter
        .native_wrapping_sub(receiver, &args, span)?
        .ok_or_else(|| {
            interpreter.runtime_error(
                span,
                format!("method 'wrappingSub' is not available on {rendered}"),
            )
        })
}

fn native_wrapping_mul(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    let rendered = receiver.render();
    interpreter
        .native_wrapping_mul(receiver, &args, span)?
        .ok_or_else(|| {
            interpreter.runtime_error(
                span,
                format!("method 'wrappingMul' is not available on {rendered}"),
            )
        })
}

fn native_checked_add(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    let rendered = receiver.render();
    interpreter
        .native_checked_add(receiver, &args, span)?
        .ok_or_else(|| {
            interpreter.runtime_error(
                span,
                format!("method 'checkedAdd' is not available on {rendered}"),
            )
        })
}

fn native_checked_sub(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    let rendered = receiver.render();
    interpreter
        .native_checked_sub(receiver, &args, span)?
        .ok_or_else(|| {
            interpreter.runtime_error(
                span,
                format!("method 'checkedSub' is not available on {rendered}"),
            )
        })
}

fn native_checked_mul(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    let rendered = receiver.render();
    interpreter
        .native_checked_mul(receiver, &args, span)?
        .ok_or_else(|| {
            interpreter.runtime_error(
                span,
                format!("method 'checkedMul' is not available on {rendered}"),
            )
        })
}

fn native_checked_pow(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    let rendered = receiver.render();
    interpreter
        .native_checked_pow(receiver, &args, span)?
        .ok_or_else(|| {
            interpreter.runtime_error(
                span,
                format!("method 'checkedPow' is not available on {rendered}"),
            )
        })
}

fn native_floor(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    let rendered = receiver.render();
    interpreter
        .native_floor(receiver, &args, span)?
        .ok_or_else(|| {
            interpreter.runtime_error(
                span,
                format!("method 'floor' is not available on {rendered}"),
            )
        })
}

fn native_ceil(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    let rendered = receiver.render();
    interpreter
        .native_ceil(receiver, &args, span)?
        .ok_or_else(|| {
            interpreter.runtime_error(
                span,
                format!("method 'ceil' is not available on {rendered}"),
            )
        })
}

fn native_round(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    let rendered = receiver.render();
    interpreter
        .native_round(receiver, &args, span)?
        .ok_or_else(|| {
            interpreter.runtime_error(
                span,
                format!("method 'round' is not available on {rendered}"),
            )
        })
}

fn native_to_int(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    let rendered = receiver.render();
    interpreter
        .native_to_int(receiver, &args, span)?
        .ok_or_else(|| {
            interpreter.runtime_error(
                span,
                format!("method 'toInt' is not available on {rendered}"),
            )
        })
}

fn native_to_float(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    let rendered = receiver.render();
    interpreter
        .native_to_float(receiver, &args, span)?
        .ok_or_else(|| {
            interpreter.runtime_error(
                span,
                format!("method 'toFloat' is not available on {rendered}"),
            )
        })
}

fn native_scale(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    let rendered = receiver.render();
    interpreter
        .native_scale(receiver, &args, span)?
        .ok_or_else(|| {
            interpreter.runtime_error(
                span,
                format!("method 'scale' is not available on {rendered}"),
            )
        })
}

fn native_to_decimal(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    let rendered = receiver.render();
    interpreter
        .native_to_decimal(receiver, &args, span)?
        .ok_or_else(|| {
            interpreter.runtime_error(
                span,
                format!("method 'toDecimal' is not available on {rendered}"),
            )
        })
}

fn native_divide(
    interpreter: &mut Interpreter<'_>,
    receiver: Value,
    args: Vec<Value>,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    let rendered = receiver.render();
    interpreter
        .native_divide(receiver, &args, span)?
        .ok_or_else(|| {
            interpreter.runtime_error(
                span,
                format!("method 'divide' is not available on {rendered}"),
            )
        })
}

pub(super) fn register(types: &mut [RuntimeType]) {
    if let Some(ty) = types
        .iter_mut()
        .find(|ty| ty.name == "Hashed" && ty.kind != TypeKind::Object)
    {
        bind(ty, "hash", native_hash);
    }
    if let Some(ty) = types
        .iter_mut()
        .find(|ty| ty.name == "Int" && ty.kind != TypeKind::Object)
    {
        bind(ty, "compareTo", native_compare_to);
        bind(ty, "sqrt", native_sqrt);
        bind(ty, "abs", native_abs);
        bind(ty, "pow", native_pow);
        bind(ty, "wrappingNeg", native_wrapping_neg);
        bind(ty, "wrappingAdd", native_wrapping_add);
        bind(ty, "wrappingSub", native_wrapping_sub);
        bind(ty, "wrappingMul", native_wrapping_mul);
        bind(ty, "checkedAdd", native_checked_add);
        bind(ty, "checkedSub", native_checked_sub);
        bind(ty, "checkedMul", native_checked_mul);
        bind(ty, "checkedPow", native_checked_pow);
        bind(ty, "toFloat", native_to_float);
        bind(ty, "toDecimal", native_to_decimal);
    }
    if let Some(ty) = types
        .iter_mut()
        .find(|ty| ty.name == "Float" && ty.kind != TypeKind::Object)
    {
        bind(ty, "compareTo", native_compare_to);
        bind(ty, "sqrt", native_sqrt);
        bind(ty, "log", native_log);
        bind(ty, "abs", native_abs);
        bind(ty, "pow", native_pow);
        bind(ty, "floor", native_floor);
        bind(ty, "ceil", native_ceil);
        bind(ty, "round", native_round);
        bind(ty, "toInt", native_to_int);
        bind(ty, "toDecimal", native_to_decimal);
    }
    if let Some(ty) = types
        .iter_mut()
        .find(|ty| ty.name == "Decimal" && ty.kind != TypeKind::Object)
    {
        bind(ty, "compareTo", native_compare_to);
        bind(ty, "abs", native_abs);
        bind(ty, "round", native_round);
        bind(ty, "divide", native_divide);
        bind(ty, "toInt", native_to_int);
        bind(ty, "toFloat", native_to_float);
        bind(ty, "scale", native_scale);
        bind(ty, "equals", native_equals);
        bind(ty, "hash", native_hash);
    }
    if let Some(ty) = types
        .iter_mut()
        .find(|ty| ty.name == "Any" && ty.kind != TypeKind::Object)
    {
        bind(ty, "toStr", native_to_str);
        bind(ty, "equals", native_equals);
    }
    if let Some(ty) = types
        .iter_mut()
        .find(|ty| ty.name == "Type" && ty.kind != TypeKind::Object)
    {
        bind(ty, "annotation", |i, r, a, s| {
            i.native_type_annotation(r, a, s)
        });
        bind(ty, "hasAnnotation", |i, r, a, s| {
            i.native_type_has_annotation(r, a, s)
        });
        bind(ty, "name", |i, r, a, s| i.native_type_name(r, a, s));
        bind(ty, "qualifiedName", |i, r, a, s| {
            i.native_type_qualified_name(r, a, s)
        });
        bind(ty, "kind", |i, r, a, s| i.native_type_kind(r, a, s));
        bind(ty, "asClass", |i, r, a, s| i.native_type_as_class(r, a, s));
        bind(ty, "asShape", |i, r, a, s| i.native_type_as_shape(r, a, s));
        bind(ty, "asUnion", |i, r, a, s| i.native_type_as_union(r, a, s));
        bind(ty, "asInterface", |i, r, a, s| {
            i.native_type_as_interface(r, a, s)
        });
        bind(ty, "asObject", |i, r, a, s| {
            i.native_type_as_object(r, a, s)
        });
        bind(ty, "asAnnotation", |i, r, a, s| {
            i.native_type_as_annotation(r, a, s)
        });
        bind(ty, "fields", |i, r, a, s| i.native_type_fields(r, a, s));
        bind(ty, "methods", |i, r, a, s| i.native_type_methods(r, a, s));
        bind(ty, "field", |i, r, a, s| i.native_type_field(r, a, s));
        bind(ty, "method", |i, r, a, s| i.native_type_method(r, a, s));
        bind(ty, "alternatives", |i, r, a, s| {
            i.native_type_alternatives(r, a, s)
        });
        bind(ty, "alternative", |i, r, a, s| {
            i.native_type_alternative(r, a, s)
        });
    }
    if let Some(ty) = types
        .iter_mut()
        .find(|ty| ty.name == "ClassType" && ty.kind != TypeKind::Object)
    {
        bind(ty, "annotation", |i, r, a, s| {
            i.native_type_annotation(r, a, s)
        });
        bind(ty, "hasAnnotation", |i, r, a, s| {
            i.native_type_has_annotation(r, a, s)
        });
        bind(ty, "name", |i, r, a, s| i.native_type_name(r, a, s));
        bind(ty, "qualifiedName", |i, r, a, s| {
            i.native_type_qualified_name(r, a, s)
        });
        bind(ty, "kind", |i, r, a, s| i.native_type_kind(r, a, s));
        bind(ty, "asClass", |i, r, a, s| i.native_type_as_class(r, a, s));
        bind(ty, "asShape", |i, r, a, s| i.native_type_as_shape(r, a, s));
        bind(ty, "asUnion", |i, r, a, s| i.native_type_as_union(r, a, s));
        bind(ty, "asInterface", |i, r, a, s| {
            i.native_type_as_interface(r, a, s)
        });
        bind(ty, "asObject", |i, r, a, s| {
            i.native_type_as_object(r, a, s)
        });
        bind(ty, "asAnnotation", |i, r, a, s| {
            i.native_type_as_annotation(r, a, s)
        });
        bind(ty, "fields", |i, r, a, s| i.native_type_fields(r, a, s));
        bind(ty, "methods", |i, r, a, s| i.native_type_methods(r, a, s));
        bind(ty, "field", |i, r, a, s| i.native_type_field(r, a, s));
        bind(ty, "method", |i, r, a, s| i.native_type_method(r, a, s));
        bind(ty, "alternatives", |i, r, a, s| {
            i.native_type_alternatives(r, a, s)
        });
        bind(ty, "alternative", |i, r, a, s| {
            i.native_type_alternative(r, a, s)
        });
    }
    if let Some(ty) = types
        .iter_mut()
        .find(|ty| ty.name == "ShapeType" && ty.kind != TypeKind::Object)
    {
        bind(ty, "annotation", |i, r, a, s| {
            i.native_type_annotation(r, a, s)
        });
        bind(ty, "hasAnnotation", |i, r, a, s| {
            i.native_type_has_annotation(r, a, s)
        });
        bind(ty, "name", |i, r, a, s| i.native_type_name(r, a, s));
        bind(ty, "qualifiedName", |i, r, a, s| {
            i.native_type_qualified_name(r, a, s)
        });
        bind(ty, "kind", |i, r, a, s| i.native_type_kind(r, a, s));
        bind(ty, "asClass", |i, r, a, s| i.native_type_as_class(r, a, s));
        bind(ty, "asShape", |i, r, a, s| i.native_type_as_shape(r, a, s));
        bind(ty, "asUnion", |i, r, a, s| i.native_type_as_union(r, a, s));
        bind(ty, "asInterface", |i, r, a, s| {
            i.native_type_as_interface(r, a, s)
        });
        bind(ty, "asObject", |i, r, a, s| {
            i.native_type_as_object(r, a, s)
        });
        bind(ty, "asAnnotation", |i, r, a, s| {
            i.native_type_as_annotation(r, a, s)
        });
        bind(ty, "fields", |i, r, a, s| i.native_type_fields(r, a, s));
        bind(ty, "methods", |i, r, a, s| i.native_type_methods(r, a, s));
        bind(ty, "field", |i, r, a, s| i.native_type_field(r, a, s));
        bind(ty, "method", |i, r, a, s| i.native_type_method(r, a, s));
        bind(ty, "alternatives", |i, r, a, s| {
            i.native_type_alternatives(r, a, s)
        });
        bind(ty, "alternative", |i, r, a, s| {
            i.native_type_alternative(r, a, s)
        });
    }
    if let Some(ty) = types
        .iter_mut()
        .find(|ty| ty.name == "UnionType" && ty.kind != TypeKind::Object)
    {
        bind(ty, "annotation", |i, r, a, s| {
            i.native_type_annotation(r, a, s)
        });
        bind(ty, "hasAnnotation", |i, r, a, s| {
            i.native_type_has_annotation(r, a, s)
        });
        bind(ty, "name", |i, r, a, s| i.native_type_name(r, a, s));
        bind(ty, "qualifiedName", |i, r, a, s| {
            i.native_type_qualified_name(r, a, s)
        });
        bind(ty, "kind", |i, r, a, s| i.native_type_kind(r, a, s));
        bind(ty, "asClass", |i, r, a, s| i.native_type_as_class(r, a, s));
        bind(ty, "asShape", |i, r, a, s| i.native_type_as_shape(r, a, s));
        bind(ty, "asUnion", |i, r, a, s| i.native_type_as_union(r, a, s));
        bind(ty, "asInterface", |i, r, a, s| {
            i.native_type_as_interface(r, a, s)
        });
        bind(ty, "asObject", |i, r, a, s| {
            i.native_type_as_object(r, a, s)
        });
        bind(ty, "asAnnotation", |i, r, a, s| {
            i.native_type_as_annotation(r, a, s)
        });
        bind(ty, "fields", |i, r, a, s| i.native_type_fields(r, a, s));
        bind(ty, "methods", |i, r, a, s| i.native_type_methods(r, a, s));
        bind(ty, "field", |i, r, a, s| i.native_type_field(r, a, s));
        bind(ty, "method", |i, r, a, s| i.native_type_method(r, a, s));
        bind(ty, "alternatives", |i, r, a, s| {
            i.native_type_alternatives(r, a, s)
        });
        bind(ty, "alternative", |i, r, a, s| {
            i.native_type_alternative(r, a, s)
        });
    }
    if let Some(ty) = types
        .iter_mut()
        .find(|ty| ty.name == "InterfaceType" && ty.kind != TypeKind::Object)
    {
        bind(ty, "annotation", |i, r, a, s| {
            i.native_type_annotation(r, a, s)
        });
        bind(ty, "hasAnnotation", |i, r, a, s| {
            i.native_type_has_annotation(r, a, s)
        });
        bind(ty, "name", |i, r, a, s| i.native_type_name(r, a, s));
        bind(ty, "qualifiedName", |i, r, a, s| {
            i.native_type_qualified_name(r, a, s)
        });
        bind(ty, "kind", |i, r, a, s| i.native_type_kind(r, a, s));
        bind(ty, "asClass", |i, r, a, s| i.native_type_as_class(r, a, s));
        bind(ty, "asShape", |i, r, a, s| i.native_type_as_shape(r, a, s));
        bind(ty, "asUnion", |i, r, a, s| i.native_type_as_union(r, a, s));
        bind(ty, "asInterface", |i, r, a, s| {
            i.native_type_as_interface(r, a, s)
        });
        bind(ty, "asObject", |i, r, a, s| {
            i.native_type_as_object(r, a, s)
        });
        bind(ty, "asAnnotation", |i, r, a, s| {
            i.native_type_as_annotation(r, a, s)
        });
        bind(ty, "fields", |i, r, a, s| i.native_type_fields(r, a, s));
        bind(ty, "methods", |i, r, a, s| i.native_type_methods(r, a, s));
        bind(ty, "field", |i, r, a, s| i.native_type_field(r, a, s));
        bind(ty, "method", |i, r, a, s| i.native_type_method(r, a, s));
        bind(ty, "alternatives", |i, r, a, s| {
            i.native_type_alternatives(r, a, s)
        });
        bind(ty, "alternative", |i, r, a, s| {
            i.native_type_alternative(r, a, s)
        });
    }
    if let Some(ty) = types
        .iter_mut()
        .find(|ty| ty.name == "ObjectType" && ty.kind != TypeKind::Object)
    {
        bind(ty, "annotation", |i, r, a, s| {
            i.native_type_annotation(r, a, s)
        });
        bind(ty, "hasAnnotation", |i, r, a, s| {
            i.native_type_has_annotation(r, a, s)
        });
        bind(ty, "name", |i, r, a, s| i.native_type_name(r, a, s));
        bind(ty, "qualifiedName", |i, r, a, s| {
            i.native_type_qualified_name(r, a, s)
        });
        bind(ty, "kind", |i, r, a, s| i.native_type_kind(r, a, s));
        bind(ty, "asClass", |i, r, a, s| i.native_type_as_class(r, a, s));
        bind(ty, "asShape", |i, r, a, s| i.native_type_as_shape(r, a, s));
        bind(ty, "asUnion", |i, r, a, s| i.native_type_as_union(r, a, s));
        bind(ty, "asInterface", |i, r, a, s| {
            i.native_type_as_interface(r, a, s)
        });
        bind(ty, "asObject", |i, r, a, s| {
            i.native_type_as_object(r, a, s)
        });
        bind(ty, "asAnnotation", |i, r, a, s| {
            i.native_type_as_annotation(r, a, s)
        });
        bind(ty, "fields", |i, r, a, s| i.native_type_fields(r, a, s));
        bind(ty, "methods", |i, r, a, s| i.native_type_methods(r, a, s));
        bind(ty, "field", |i, r, a, s| i.native_type_field(r, a, s));
        bind(ty, "method", |i, r, a, s| i.native_type_method(r, a, s));
        bind(ty, "alternatives", |i, r, a, s| {
            i.native_type_alternatives(r, a, s)
        });
        bind(ty, "alternative", |i, r, a, s| {
            i.native_type_alternative(r, a, s)
        });
    }
    if let Some(ty) = types
        .iter_mut()
        .find(|ty| ty.name == "AnnotationType" && ty.kind != TypeKind::Object)
    {
        bind(ty, "annotation", |i, r, a, s| {
            i.native_type_annotation(r, a, s)
        });
        bind(ty, "hasAnnotation", |i, r, a, s| {
            i.native_type_has_annotation(r, a, s)
        });
        bind(ty, "name", |i, r, a, s| i.native_type_name(r, a, s));
        bind(ty, "qualifiedName", |i, r, a, s| {
            i.native_type_qualified_name(r, a, s)
        });
        bind(ty, "kind", |i, r, a, s| i.native_type_kind(r, a, s));
        bind(ty, "asClass", |i, r, a, s| i.native_type_as_class(r, a, s));
        bind(ty, "asShape", |i, r, a, s| i.native_type_as_shape(r, a, s));
        bind(ty, "asUnion", |i, r, a, s| i.native_type_as_union(r, a, s));
        bind(ty, "asInterface", |i, r, a, s| {
            i.native_type_as_interface(r, a, s)
        });
        bind(ty, "asObject", |i, r, a, s| {
            i.native_type_as_object(r, a, s)
        });
        bind(ty, "asAnnotation", |i, r, a, s| {
            i.native_type_as_annotation(r, a, s)
        });
        bind(ty, "fields", |i, r, a, s| i.native_type_fields(r, a, s));
        bind(ty, "methods", |i, r, a, s| i.native_type_methods(r, a, s));
        bind(ty, "field", |i, r, a, s| i.native_type_field(r, a, s));
        bind(ty, "method", |i, r, a, s| i.native_type_method(r, a, s));
        bind(ty, "alternatives", |i, r, a, s| {
            i.native_type_alternatives(r, a, s)
        });
        bind(ty, "alternative", |i, r, a, s| {
            i.native_type_alternative(r, a, s)
        });
    }
    if let Some(ty) = types
        .iter_mut()
        .find(|ty| ty.name == "Field" && ty.kind != TypeKind::Object)
    {
        bind(ty, "annotation", |i, r, a, s| {
            i.native_field_annotation(r, a, s)
        });
        bind(ty, "hasAnnotation", |i, r, a, s| {
            i.native_field_has_annotation(r, a, s)
        });
        bind(ty, "name", |i, r, a, s| i.native_field_name(r, a, s));
        bind(ty, "fieldType", |i, r, a, s| {
            i.native_field_field_type(r, a, s)
        });
        bind(ty, "isPrivate", |i, r, a, s| {
            i.native_field_is_private(r, a, s)
        });
    }
    if let Some(ty) = types
        .iter_mut()
        .find(|ty| ty.name == "Method" && ty.kind != TypeKind::Object)
    {
        bind(ty, "annotation", |i, r, a, s| {
            i.native_method_annotation(r, a, s)
        });
        bind(ty, "hasAnnotation", |i, r, a, s| {
            i.native_method_has_annotation(r, a, s)
        });
        bind(ty, "name", |i, r, a, s| i.native_method_name(r, a, s));
        bind(ty, "params", |i, r, a, s| i.native_method_params(r, a, s));
        bind(ty, "returnType", |i, r, a, s| {
            i.native_method_return_type(r, a, s)
        });
    }
    if let Some(ty) = types
        .iter_mut()
        .find(|ty| ty.name == "Param" && ty.kind != TypeKind::Object)
    {
        bind(ty, "name", |i, r, a, s| i.native_param_name(r, a, s));
        bind(ty, "paramType", |i, r, a, s| {
            i.native_param_param_type(r, a, s)
        });
    }
    if let Some(ty) = types
        .iter_mut()
        .find(|ty| ty.name == "UnionAlternative" && ty.kind != TypeKind::Object)
    {
        bind(ty, "annotation", |i, r, a, s| {
            i.native_alternative_annotation(r, a, s)
        });
        bind(ty, "hasAnnotation", |i, r, a, s| {
            i.native_alternative_has_annotation(r, a, s)
        });
        bind(ty, "name", |i, r, a, s| i.native_alternative_name(r, a, s));
        bind(ty, "fields", |i, r, a, s| {
            i.native_alternative_fields(r, a, s)
        });
    }
    if let Some(ty) = types
        .iter_mut()
        .find(|ty| ty.name == "OS" && ty.kind == TypeKind::Object)
    {
        bind(ty, "print", |i, r, a, s| i.native_os_print(r, a, s));
        bind(ty, "println", |i, r, a, s| i.native_os_println(r, a, s));
        bind(ty, "printf", |i, r, a, s| i.native_os_printf(r, a, s));
        bind(ty, "args", |i, r, a, s| i.native_os_args(r, a, s));
        bind(ty, "env", |i, r, a, s| i.native_os_env(r, a, s));
    }
    if let Some(ty) = types
        .iter_mut()
        .find(|ty| ty.name == "File" && ty.kind == TypeKind::Object)
    {
        bind(ty, "readBytes", |i, r, a, s| {
            i.native_file_read_bytes(r, a, s)
        });
        bind(ty, "readText", |i, r, a, s| {
            i.native_file_read_text(r, a, s)
        });
        bind(ty, "open", |i, r, a, s| i.native_file_open(r, a, s));
        bind(ty, "openText", |i, r, a, s| {
            i.native_file_open_text(r, a, s)
        });
    }
    if let Some(ty) = types
        .iter_mut()
        .find(|ty| ty.name == "FileStream" && ty.kind != TypeKind::Object)
    {
        bind(ty, "path", |i, r, a, s| i.native_stream_path(r, a, s));
        bind(ty, "closed", |i, r, a, s| i.native_stream_closed(r, a, s));
        bind(ty, "position", |i, r, a, s| {
            i.native_stream_position(r, a, s)
        });
        bind(ty, "read", |i, r, a, s| i.native_stream_read(r, a, s));
        bind(ty, "readToEnd", |i, r, a, s| {
            i.native_stream_read_to_end(r, a, s)
        });
        bind(ty, "seek", |i, r, a, s| i.native_stream_seek(r, a, s));
        bind(ty, "close", |i, r, a, s| i.native_stream_close(r, a, s));
    }
    if let Some(ty) = types
        .iter_mut()
        .find(|ty| ty.name == "TextFileReader" && ty.kind != TypeKind::Object)
    {
        bind(ty, "path", |i, r, a, s| i.native_reader_path(r, a, s));
        bind(ty, "closed", |i, r, a, s| i.native_reader_closed(r, a, s));
        bind(ty, "readLine", |i, r, a, s| {
            i.native_reader_read_line(r, a, s)
        });
        bind(ty, "readToEnd", |i, r, a, s| {
            i.native_reader_read_to_end(r, a, s)
        });
        bind(ty, "close", |i, r, a, s| i.native_reader_close(r, a, s));
    }
    if let Some(ty) = types
        .iter_mut()
        .find(|ty| ty.name == "Math" && ty.kind == TypeKind::Object)
    {
        bind(ty, "min", |i, r, a, s| i.native_math_min(r, a, s));
        bind(ty, "max", |i, r, a, s| i.native_math_max(r, a, s));
    }
    if let Some(ty) = types
        .iter_mut()
        .find(|ty| ty.name == "Json" && ty.kind == TypeKind::Object)
    {
        bind(ty, "str", |i, r, a, s| i.native_json_str(r, a, s));
        bind(ty, "int", |i, r, a, s| i.native_json_int(r, a, s));
        bind(ty, "float", |i, r, a, s| i.native_json_float(r, a, s));
        bind(ty, "bool", |i, r, a, s| i.native_json_bool(r, a, s));
        bind(ty, "nil", |i, r, a, s| i.native_json_nil(r, a, s));
        bind(ty, "field", |i, r, a, s| i.native_json_field(r, a, s));
        bind(ty, "array", |i, r, a, s| i.native_json_array(r, a, s));
        bind(ty, "obj", |i, r, a, s| i.native_json_obj(r, a, s));
        bind(ty, "encode", |i, r, a, s| i.native_json_encode(r, a, s));
        bind(ty, "stringify", |i, r, a, s| {
            i.native_json_stringify(r, a, s)
        });
        bind(ty, "textContent", |i, r, a, s| {
            i.native_json_text_content(r, a, s)
        });
        bind(ty, "decode", |i, r, a, s| i.native_json_decode(r, a, s));
    }
    if let Some(ty) = types
        .iter_mut()
        .find(|ty| ty.name == "Regex" && ty.kind == TypeKind::Object)
    {
        bind(ty, "compile", |i, r, a, s| {
            i.native_regex_static_compile(r, a, s)
        });
        bind(ty, "escape", |i, r, a, s| {
            i.native_regex_static_escape(r, a, s)
        });
    }
    if let Some(ty) = types
        .iter_mut()
        .find(|ty| ty.name == "Regex" && ty.kind != TypeKind::Object)
    {
        bind(ty, "contains", |i, r, a, s| {
            i.native_regex_contains(r, a, s)
        });
        bind(ty, "matches", |i, r, a, s| i.native_regex_matches(r, a, s));
        bind(ty, "find", |i, r, a, s| i.native_regex_find(r, a, s));
        bind(ty, "fullMatch", |i, r, a, s| {
            i.native_regex_full_match(r, a, s)
        });
        bind(ty, "findAll", |i, r, a, s| i.native_regex_find_all(r, a, s));
    }
    if let Some(ty) = types
        .iter_mut()
        .find(|ty| ty.name == "RegexMatch" && ty.kind != TypeKind::Object)
    {
        bind(ty, "text", |i, r, a, s| i.native_regex_match_text(r, a, s));
        bind(ty, "start", |i, r, a, s| {
            i.native_regex_match_start(r, a, s)
        });
        bind(ty, "end", |i, r, a, s| i.native_regex_match_end(r, a, s));
        bind(ty, "groups", |i, r, a, s| {
            i.native_regex_match_groups(r, a, s)
        });
        bind(ty, "groupMap", |i, r, a, s| {
            i.native_regex_match_group_map(r, a, s)
        });
        bind(ty, "group", |i, r, a, s| {
            i.native_regex_match_group(r, a, s)
        });
    }
    if let Some(ty) = types
        .iter_mut()
        .find(|ty| ty.name == "Vector" && ty.kind == TypeKind::Object)
    {
        bind(ty, "fill", |i, r, a, s| i.native_vector_fill(r, a, s));
        bind(ty, "generate", |i, r, a, s| {
            i.native_vector_generate(r, a, s)
        });
    }
    if let Some(ty) = types
        .iter_mut()
        .find(|ty| ty.name == "Array" && ty.kind == TypeKind::Object)
    {
        bind(ty, "fill", |i, r, a, s| i.native_array_fill(r, a, s));
        bind(ty, "generate", |i, r, a, s| {
            i.native_array_generate(r, a, s)
        });
    }
    if let Some(ty) = types
        .iter_mut()
        .find(|ty| ty.name == "Vector" && ty.kind == TypeKind::Object)
    {
        bind(ty, "from", |i, r, a, s| i.native_vector_from(r, a, s));
    }
    if let Some(ty) = types
        .iter_mut()
        .find(|ty| ty.name == "Set" && ty.kind == TypeKind::Object)
    {
        bind(ty, "from", |i, r, a, s| i.native_set_from(r, a, s));
    }
    if let Some(ty) = types
        .iter_mut()
        .find(|ty| ty.name == "Int" && ty.kind == TypeKind::Object)
    {
        bind(ty, "parse", |i, r, a, s| i.native_int_parse(r, a, s));
    }
    if let Some(ty) = types
        .iter_mut()
        .find(|ty| ty.name == "Float" && ty.kind == TypeKind::Object)
    {
        bind(ty, "parse", |i, r, a, s| i.native_float_parse(r, a, s));
    }
    if let Some(ty) = types
        .iter_mut()
        .find(|ty| ty.name == "Decimal" && ty.kind == TypeKind::Object)
    {
        bind(ty, "parse", |i, r, a, s| i.native_decimal_parse(r, a, s));
    }
    if let Some(ty) = types
        .iter_mut()
        .find(|ty| ty.name == "Option" && ty.kind == TypeKind::Object)
    {
        bind(ty, "when", |i, r, a, s| i.native_option_when(r, a, s));
    }
    if let Some(ty) = types
        .iter_mut()
        .find(|ty| ty.name == "Iterator" && ty.kind != TypeKind::Object)
    {
        bind(ty, "hasNext", |i, r, a, s| {
            i.native_iterator_has_next(r, a, s)
        });
        bind(ty, "next", |i, r, a, s| i.native_iterator_next(r, a, s));
        bind(ty, "zip", |i, r, a, s| i.native_iterator_zip(r, a, s));
        bind(ty, "zipWithIndex", |i, r, a, s| {
            i.native_iterator_zip_with_index(r, a, s)
        });
    }
}
