mod either_type;
mod list_type;
mod map_type;
mod option_type;
mod result_type;
mod set_type;
mod str_type;

use crate::{
    Diagnostic, Span,
    interpreter::{Interpreter, Value},
    ir,
};

use super::{RuntimeType, RuntimeTypeId};
use crate::runtime::{RuntimeMethod, RuntimeMethodSlot, RuntimeMethodTarget};

pub(super) type BuiltinMethodFn =
    for<'a> fn(&mut Interpreter<'a>, Value, Vec<Value>, Option<Span>) -> Result<Value, Diagnostic>;

pub(super) fn builtin_method(
    slot: usize,
    name: &str,
    params: Vec<ir::Type>,
    target: BuiltinMethodFn,
) -> RuntimeMethod {
    RuntimeMethod {
        slot: RuntimeMethodSlot(slot),
        name: name.to_string(),
        target: RuntimeMethodTarget::Builtin(target),
        param_names: (0..params.len())
            .map(|index| format!("_{}", index + 1))
            .collect(),
        params,
        return_ty: ir::Type::Unknown,
    }
}

pub(super) fn force_lazy_arg(
    interpreter: &mut Interpreter<'_>,
    value: Value,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    if interpreter.value_is_zero_arg_closure(&value) {
        interpreter.invoke_value(value, Vec::new(), span)
    } else {
        Ok(value)
    }
}

/// Returns builtin runtime types with placeholder ids.
///
/// The main runtime builder assigns stable `RuntimeTypeId`s after combining
/// these definitions with user-defined IR-derived types.
pub(super) fn builtin_types() -> Vec<RuntimeType> {
    vec![
        empty_builtin_type("Bool"),
        empty_builtin_type("Float"),
        empty_builtin_type("Float32"),
        empty_builtin_type("Int"),
        empty_builtin_type("Int32"),
        empty_builtin_type("Rune"),
        str_type::define(),
        option_type::define(),
        result_type::define(),
        either_type::define(),
        list_type::define(),
        set_type::define(),
        map_type::define(),
    ]
}

fn empty_builtin_type(name: &str) -> RuntimeType {
    RuntimeType {
        id: RuntimeTypeId(usize::MAX),
        ir_type_id: None,
        kind: crate::ast::TypeKind::Class,
        name: name.to_string(),
        fields: Vec::new(),
        field_init: None,
        methods: Vec::new(),
        enum_cases: Vec::new(),
        with_bounds: Vec::new(),
    }
}

pub(super) fn assign_type_ids(types: &mut [RuntimeType], start: usize) {
    for (index, ty) in types.iter_mut().enumerate() {
        ty.id = RuntimeTypeId(start + index);
    }
}
