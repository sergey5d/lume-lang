use std::collections::HashMap;

use crate::{ast::TypeKind, ir};

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
}

/// Runtime method metadata. Overloads occupy distinct slots.
#[derive(Debug, Clone)]
pub struct RuntimeMethod {
    pub slot: RuntimeMethodSlot,
    pub name: String,
    pub function: ir::FunctionId,
    pub params: Vec<ir::Type>,
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
    pub ir_type_id: Option<ir::TypeId>,
    pub kind: TypeKind,
    pub name: String,
    pub fields: Vec<RuntimeField>,
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

impl RuntimeProgram {
    pub fn from_ir(program: &ir::Program) -> Self {
        let mut types = Vec::with_capacity(program.types.len());
        let mut by_name_kind = HashMap::with_capacity(program.types.len());
        let mut by_ir_type = vec![RuntimeTypeId(usize::MAX); program.types.len()];

        for ir_ty in &program.types {
            let runtime_id = RuntimeTypeId(types.len());
            by_name_kind.insert((ir_ty.name.clone(), ir_ty.kind), runtime_id);
            if ir_ty.id.0 < by_ir_type.len() {
                by_ir_type[ir_ty.id.0] = runtime_id;
            }

            let fields = ir_ty
                .fields
                .iter()
                .enumerate()
                .map(|(index, field)| RuntimeField {
                    slot: RuntimeFieldSlot(index),
                    name: field.name.clone(),
                    ty: field.ty.clone(),
                    mutable: field.mutable,
                })
                .collect();

            let methods = ir_ty
                .methods
                .iter()
                .enumerate()
                .filter_map(|(index, function_id)| {
                    let function = program.function(*function_id)?;
                    let params = function
                        .params
                        .iter()
                        .filter_map(|local_id| function.locals.get(local_id.0))
                        .map(|local| local.ty.clone())
                        .collect();
                    Some(RuntimeMethod {
                        slot: RuntimeMethodSlot(index),
                        name: function.name.clone(),
                        function: *function_id,
                        params,
                    })
                })
                .collect();

            let enum_cases = ir_ty
                .enum_cases
                .iter()
                .enumerate()
                .map(|(index, case)| RuntimeEnumCase {
                    id: RuntimeEnumCaseId(index),
                    name: case.name.clone(),
                    fields: case
                        .fields
                        .iter()
                        .enumerate()
                        .map(|(field_index, field)| RuntimeField {
                            slot: RuntimeFieldSlot(field_index),
                            name: field.name.clone(),
                            ty: field.ty.clone(),
                            mutable: field.mutable,
                        })
                        .collect(),
                })
                .collect();

            types.push(RuntimeType {
                id: runtime_id,
                ir_type_id: Some(ir_ty.id),
                kind: ir_ty.kind,
                name: ir_ty.name.clone(),
                fields,
                methods,
                enum_cases,
                with_bounds: Vec::new(),
            });
        }

        for (index, ir_ty) in program.types.iter().enumerate() {
            let mut bounds = Vec::new();
            for bound in &ir_ty.with_bounds {
                let ir::Type::Named { name, .. } = bound else {
                    continue;
                };
                if let Some(interface_id) = by_name_kind
                    .get(&(name.clone(), TypeKind::Interface))
                    .copied()
                    .or_else(|| {
                        by_name_kind
                            .iter()
                            .find_map(|((bound_name, _), id)| (bound_name == name).then_some(*id))
                    })
                {
                    bounds.push(interface_id);
                }
            }
            types[index].with_bounds = bounds;
        }

        Self {
            types,
            by_name_kind,
            by_ir_type,
        }
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
                    .map(|method| method.function)
                    .collect()
            })
            .unwrap_or_default()
    }
}
