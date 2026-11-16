use std::collections::HashMap;

use crate::{
    ast::{TypeKind, Visibility},
    ir,
};

pub(crate) type BuiltinMethodFn =
    for<'a> fn(
        &mut crate::interpreter::Interpreter<'a>,
        crate::interpreter::Value,
        Vec<crate::interpreter::Value>,
        Option<crate::source::Span>,
    ) -> Result<crate::interpreter::Value, crate::Diagnostic>;

/// Stable id for one runtime-visible type metadata entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RuntimeTypeId(pub usize);

/// Stable slot index for one runtime field layout entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RuntimeFieldSlot(pub usize);

/// Stable slot index for one lowered method implementation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RuntimeMethodSlot(pub usize);

/// Stable id for one enum case inside a runtime type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RuntimeEnumCaseId(pub usize);

/// Runtime field metadata derived from an IR field declaration.
#[derive(Debug, Clone)]
pub struct RuntimeField {
    pub slot: RuntimeFieldSlot,
    pub name: String,
    pub ty: ir::Type,
    pub mutable: bool,
    pub from_mixin: bool,
    pub visibility: Visibility,
    pub has_initializer: bool,
    pub initializer: Option<ir::Constant>,
}

/// Runtime method metadata. Overloads occupy distinct slots.
#[derive(Debug, Clone)]
pub struct RuntimeMethod {
    pub slot: RuntimeMethodSlot,
    pub name: String,
    pub(crate) target: RuntimeMethodTarget,
    pub param_names: Vec<String>,
    pub params: Vec<ir::Type>,
    pub return_ty: ir::Type,
    pub(crate) kind: RuntimeMethodKind,
    pub(crate) variadic: bool,
    pub(crate) default_count: usize,
    pub(crate) reified_params: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RuntimeMethodKind {
    Instance,
    Getter,
    Static,
}

/// A runtime method can either jump into lowered IR or invoke a builtin host
/// implementation registered by the runtime.
#[derive(Clone, Copy)]
pub(crate) enum RuntimeMethodTarget {
    Ir(ir::FunctionId),
    Builtin(BuiltinMethodFn),
}

impl std::fmt::Debug for RuntimeMethodTarget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Ir(function) => f.debug_tuple("Ir").field(function).finish(),
            Self::Builtin(_) => f.write_str("Builtin(<fn>)"),
        }
    }
}

/// Runtime enum-case metadata with a fixed payload layout.
#[derive(Debug, Clone)]
pub struct RuntimeEnumCase {
    pub id: RuntimeEnumCaseId,
    pub name: String,
    pub fields: Vec<RuntimeField>,
}

/// Runtime type metadata used by the interpreter hot path.
#[derive(Debug, Clone)]
pub struct RuntimeType {
    pub id: RuntimeTypeId,
    pub(crate) native: bool,
    pub ir_type_id: Option<ir::TypeId>,
    pub kind: TypeKind,
    pub name: String,
    pub fields: Vec<RuntimeField>,
    pub field_init: Option<ir::FunctionId>,
    pub mixin_init: Option<ir::FunctionId>,
    pub methods: Vec<RuntimeMethod>,
    pub enum_cases: Vec<RuntimeEnumCase>,
    pub with_bounds: Vec<RuntimeTypeId>,
}

/// Execution-oriented metadata built from one lowered IR program.
#[derive(Debug, Clone)]
pub struct RuntimeProgram {
    pub types: Vec<RuntimeType>,
    by_name_kind: HashMap<(String, TypeKind), RuntimeTypeId>,
    by_ir_type: Vec<RuntimeTypeId>,
}

/// Stable lookup indexes built before we materialize runtime type metadata.
///
/// The runtime keeps both "by name/kind" and "by IR type id" views so the
/// interpreter can move between symbolic lookups and dense ids cheaply.
#[derive(Debug)]
struct RuntimeIndexes {
    by_name_kind: HashMap<(String, TypeKind), RuntimeTypeId>,
    by_ir_type: Vec<RuntimeTypeId>,
}

impl RuntimeProgram {
    pub fn from_ir(program: &ir::Program) -> Self {
        let mut types = crate::stdlib::native::runtime_types();
        crate::stdlib::native::assign_type_ids(&mut types, 0);
        let mut by_name_kind = HashMap::with_capacity(program.types.len() + types.len());
        for ty in &types {
            by_name_kind.insert((ty.name.clone(), ty.kind), ty.id);
        }

        // First assign stable runtime ids for every lowered type so later passes
        // can refer to types densely instead of rediscovering them by name.
        // Synthetic primitive extension buckets merge into existing builtin
        // runtime types instead of shadowing methods like Str.size().
        let indexes = Self::build_indexes(program, types.len(), &by_name_kind);
        by_name_kind.extend(indexes.by_name_kind.clone());

        // Then convert each IR type definition into the compact runtime shape
        // the interpreter uses on its hot path.
        for ir_ty in &program.types {
            let runtime_id = indexes
                .by_ir_type
                .get(ir_ty.id.0)
                .copied()
                .unwrap_or(RuntimeTypeId(usize::MAX));
            if runtime_id.0 < types.len() {
                let mut lowered = Self::build_runtime_type(program, ir_ty, runtime_id);
                let native = &mut types[runtime_id.0];
                // Shared declaration bodies are markers, not executable IR.
                // Unbound capabilities must remain unsupported at runtime.
                if Self::is_native_declaration(ir_ty) {
                    lowered.methods.clear();
                }
                lowered.methods.retain(|method| {
                    !native.methods.iter().any(|existing| {
                        existing.name == method.name && existing.params.len() == method.params.len()
                    })
                });
                native.methods.extend(lowered.methods);
                native.ir_type_id = Some(ir_ty.id);
                if !lowered.fields.is_empty() {
                    native.fields = lowered.fields;
                }
                if lowered.field_init.is_some() {
                    native.field_init = lowered.field_init;
                }
                if lowered.mixin_init.is_some() {
                    native.mixin_init = lowered.mixin_init;
                }
                if !lowered.enum_cases.is_empty() {
                    native.enum_cases = lowered.enum_cases;
                }
                continue;
            }
            types.push(Self::build_runtime_type(program, ir_ty, runtime_id));
        }

        // Finally resolve interface/with-bound metadata once all runtime ids are
        // known, so aggregate instances can answer type-relationship questions
        // without rescanning the IR graph.
        Self::populate_with_bounds(&mut types, program, &indexes.by_ir_type, &by_name_kind);

        for ty in &mut types {
            for (index, method) in ty.methods.iter_mut().enumerate() {
                method.slot = RuntimeMethodSlot(index);
            }
        }

        Self {
            types,
            by_name_kind,
            by_ir_type: indexes.by_ir_type,
        }
    }

    fn build_indexes(
        program: &ir::Program,
        start_index: usize,
        builtin_by_name_kind: &HashMap<(String, TypeKind), RuntimeTypeId>,
    ) -> RuntimeIndexes {
        let mut by_name_kind = HashMap::with_capacity(program.types.len());
        let mut by_ir_type = vec![RuntimeTypeId(usize::MAX); program.types.len()];
        let mut next_index = start_index;

        for ir_ty in &program.types {
            let native = builtin_by_name_kind
                .get(&(ir_ty.name.clone(), ir_ty.kind))
                .copied()
                .filter(|_| Self::is_native_declaration(ir_ty))
                .or_else(|| {
                    // Primitive extensions lower into Class buckets even when
                    // the public native contract is an Interface.
                    if ir_ty.kind != TypeKind::Class
                        || !crate::stdlib::is_primitive(&ir_ty.name)
                        || !ir_ty.fields.is_empty()
                        || !ir_ty.enum_cases.is_empty()
                        || ir_ty.field_init.is_some()
                    {
                        return None;
                    }
                    builtin_by_name_kind
                        .get(&(ir_ty.name.clone(), TypeKind::Interface))
                        .copied()
                });
            let runtime_id = if let Some(id) = native {
                by_name_kind.insert((ir_ty.name.clone(), ir_ty.kind), id);
                id
            } else {
                let id = RuntimeTypeId(next_index);
                next_index += 1;
                by_name_kind.insert((ir_ty.name.clone(), ir_ty.kind), id);
                id
            };
            if ir_ty.id.0 < by_ir_type.len() {
                by_ir_type[ir_ty.id.0] = runtime_id;
            }
        }

        RuntimeIndexes {
            by_name_kind,
            by_ir_type,
        }
    }

    fn build_runtime_type(
        program: &ir::Program,
        ir_ty: &ir::TypeDef,
        runtime_id: RuntimeTypeId,
    ) -> RuntimeType {
        RuntimeType {
            id: runtime_id,
            native: false,
            ir_type_id: Some(ir_ty.id),
            kind: ir_ty.kind,
            name: ir_ty.name.clone(),
            fields: Self::build_fields(&ir_ty.fields),
            field_init: ir_ty.field_init,
            mixin_init: ir_ty.mixin_init,
            methods: Self::build_methods(program, &ir_ty.methods),
            enum_cases: Self::build_enum_cases(&ir_ty.enum_cases),
            with_bounds: Vec::new(),
        }
    }

    fn is_native_declaration(ty: &ir::TypeDef) -> bool {
        crate::stdlib::type_declaration(&ty.name, ty.kind)
            .is_some_and(|declaration| ty.span == Some(declaration.span))
    }

    pub(crate) fn native_type_by_name_kind(
        &self,
        name: &str,
        kind: TypeKind,
    ) -> Option<&RuntimeType> {
        self.types
            .iter()
            .find(|ty| ty.native && ty.name == name && ty.kind == kind)
    }

    pub(crate) fn active_type_is_native(&self, name: &str, kind: TypeKind) -> bool {
        self.type_by_name_kind(name, kind)
            .is_some_and(|ty| ty.native)
    }

    pub(crate) fn active_name_is_native(&self, name: &str) -> bool {
        let mut found = false;
        for ((candidate, _), id) in &self.by_name_kind {
            if candidate == name {
                found = true;
                if !self.types[id.0].native {
                    return false;
                }
            }
        }
        found
    }

    pub(crate) fn source_type_named(&self, name: &str) -> bool {
        self.by_name_kind
            .iter()
            .any(|((candidate, _), id)| candidate == name && !self.types[id.0].native)
    }

    fn build_fields(fields: &[ir::Field]) -> Vec<RuntimeField> {
        fields
            .iter()
            .enumerate()
            .map(|(index, field)| RuntimeField {
                slot: RuntimeFieldSlot(index),
                name: field.name.clone(),
                ty: field.ty.clone(),
                mutable: field.mutable,
                from_mixin: field.from_mixin,
                visibility: field.visibility,
                has_initializer: field.has_initializer,
                initializer: field.initializer.clone(),
            })
            .collect()
    }

    fn build_methods(program: &ir::Program, methods: &[ir::FunctionId]) -> Vec<RuntimeMethod> {
        methods
            .iter()
            .enumerate()
            .filter_map(|(index, function_id)| {
                let function = program.function(*function_id)?;
                let params = function
                    .params
                    .iter()
                    .filter_map(|local_id| function.locals.get(local_id.0))
                    .filter(|local| !is_reified_type_param_local(&local.name))
                    .map(|local| local.ty.clone())
                    .collect();
                let param_names = function
                    .params
                    .iter()
                    .filter_map(|local_id| function.locals.get(local_id.0))
                    .filter(|local| !is_reified_type_param_local(&local.name))
                    .map(|local| local.name.clone())
                    .collect();
                Some(RuntimeMethod {
                    slot: RuntimeMethodSlot(index),
                    name: function.name.clone(),
                    target: RuntimeMethodTarget::Ir(*function_id),
                    param_names,
                    params,
                    return_ty: function.return_ty.clone(),
                    kind: if function.getter {
                        RuntimeMethodKind::Getter
                    } else {
                        RuntimeMethodKind::Instance
                    },
                    variadic: function.param_variadic.iter().any(|value| *value),
                    default_count: function
                        .param_defaults
                        .iter()
                        .filter(|value| value.is_some())
                        .count(),
                    reified_params: function.reified_type_params.len(),
                })
            })
            .collect()
    }

    fn build_enum_cases(cases: &[ir::EnumCase]) -> Vec<RuntimeEnumCase> {
        cases
            .iter()
            .enumerate()
            .map(|(index, case)| RuntimeEnumCase {
                id: RuntimeEnumCaseId(index),
                name: case.name.clone(),
                fields: Self::build_fields(&case.fields),
            })
            .collect()
    }

    fn populate_with_bounds(
        types: &mut [RuntimeType],
        program: &ir::Program,
        by_ir_type: &[RuntimeTypeId],
        by_name_kind: &HashMap<(String, TypeKind), RuntimeTypeId>,
    ) {
        for ir_ty in &program.types {
            let Some(runtime_id) = by_ir_type.get(ir_ty.id.0).copied() else {
                continue;
            };
            let bounds = ir_ty
                .with_bounds
                .iter()
                .filter_map(|bound| Self::resolve_bound_type_id(bound, by_name_kind))
                .collect::<Vec<_>>();
            for bound in bounds {
                if !types[runtime_id.0].with_bounds.contains(&bound) {
                    types[runtime_id.0].with_bounds.push(bound);
                }
            }
        }
    }

    fn resolve_bound_type_id(
        bound: &ir::Type,
        by_name_kind: &HashMap<(String, TypeKind), RuntimeTypeId>,
    ) -> Option<RuntimeTypeId> {
        let ir::Type::Named { name, .. } = bound else {
            return None;
        };

        by_name_kind
            .get(&(name.clone(), TypeKind::Interface))
            .copied()
            .or_else(|| {
                by_name_kind
                    .iter()
                    .find_map(|((bound_name, _), id)| (bound_name == name).then_some(*id))
            })
    }

    pub fn type_by_id(&self, id: RuntimeTypeId) -> Option<&RuntimeType> {
        self.types.get(id.0)
    }

    pub fn type_by_ir_id(&self, id: ir::TypeId) -> Option<&RuntimeType> {
        let runtime_id = self.by_ir_type.get(id.0).copied()?;
        self.type_by_id(runtime_id)
    }

    pub fn type_id_for_ir_id(&self, id: ir::TypeId) -> Option<RuntimeTypeId> {
        self.by_ir_type.get(id.0).copied()
    }

    pub fn type_by_name_kind(&self, name: &str, kind: TypeKind) -> Option<&RuntimeType> {
        self.by_name_kind
            .get(&(name.to_string(), kind))
            .and_then(|id| self.type_by_id(*id))
    }

    pub fn type_id_by_name_kind(&self, name: &str, kind: TypeKind) -> Option<RuntimeTypeId> {
        self.by_name_kind.get(&(name.to_string(), kind)).copied()
    }

    pub fn type_id_by_name_any_kind(&self, name: &str) -> Option<RuntimeTypeId> {
        [
            TypeKind::Class,
            TypeKind::Record,
            TypeKind::Enum,
            TypeKind::Interface,
            TypeKind::Object,
            TypeKind::Annotation,
        ]
        .into_iter()
        .find_map(|kind| self.type_id_by_name_kind(name, kind))
    }

    pub fn field_index(
        &self,
        type_id: RuntimeTypeId,
        case_id: Option<RuntimeEnumCaseId>,
        name: &str,
    ) -> Option<usize> {
        let ty = self.type_by_id(type_id)?;
        let fields = match case_id {
            Some(case_id) => &ty.enum_cases.get(case_id.0)?.fields,
            None => &ty.fields,
        };
        fields.iter().position(|field| field.name == name)
    }

    pub fn field_name(
        &self,
        type_id: RuntimeTypeId,
        case_id: Option<RuntimeEnumCaseId>,
        index: usize,
    ) -> Option<&str> {
        let ty = self.type_by_id(type_id)?;
        let fields = match case_id {
            Some(case_id) => &ty.enum_cases.get(case_id.0)?.fields,
            None => &ty.fields,
        };
        fields.get(index).map(|field| field.name.as_str())
    }

    pub fn enum_case_by_name(
        &self,
        type_id: RuntimeTypeId,
        case_name: &str,
    ) -> Option<&RuntimeEnumCase> {
        self.type_by_id(type_id)?
            .enum_cases
            .iter()
            .find(|case| case.name == case_name)
    }

    pub fn methods_named(&self, type_id: RuntimeTypeId, name: &str) -> Vec<ir::FunctionId> {
        self.type_by_id(type_id)
            .map(|ty| {
                ty.methods
                    .iter()
                    .filter(|method| method.name == name)
                    .filter_map(|method| match method.target {
                        RuntimeMethodTarget::Ir(function) => Some(function),
                        RuntimeMethodTarget::Builtin(_) => None,
                    })
                    .collect()
            })
            .unwrap_or_default()
    }
}

fn is_reified_type_param_local(name: &str) -> bool {
    name.starts_with("__type_")
}
