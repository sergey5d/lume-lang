use crate::{ast::TypeKind, backend::externals::ExternalDescriptors, ir};

#[derive(Debug, Clone, Default)]
pub struct BackendDescriptors {
    pub module: Option<DescriptorModule>,
    pub globals: Vec<DescriptorGlobal>,
    pub functions: Vec<DescriptorFunction>,
    pub types: Vec<DescriptorType>,
}

#[derive(Debug, Clone)]
pub struct DescriptorModule {
    pub name: String,
}

#[derive(Debug, Clone)]
pub struct DescriptorGlobal {
    pub name: String,
    pub ty: String,
    pub mutable: bool,
}

#[derive(Debug, Clone)]
pub struct DescriptorFunction {
    pub name: String,
    pub owner: Option<String>,
    pub origin: DescriptorOrigin,
    pub params: Vec<String>,
    pub return_ty: String,
    pub block_count: usize,
}

#[derive(Debug, Clone)]
pub struct DescriptorType {
    pub name: String,
    pub kind: String,
    pub origin: DescriptorOrigin,
    pub type_params: Vec<String>,
    pub fields: Vec<DescriptorField>,
    pub method_names: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DescriptorOrigin {
    Lume,
    Java { qualified_name: String },
}

#[derive(Debug, Clone)]
pub struct DescriptorField {
    pub name: String,
    pub ty: String,
    pub mutable: bool,
}

impl BackendDescriptors {
    pub fn from_ir(program: &ir::Program) -> Self {
        Self::from_ir_and_externals(program, &ExternalDescriptors::default())
    }

    pub fn from_ir_and_externals(program: &ir::Program, externals: &ExternalDescriptors) -> Self {
        let module = program
            .module
            .as_ref()
            .map(|name| DescriptorModule { name: name.clone() });

        let globals = program
            .globals
            .iter()
            .map(|global| DescriptorGlobal {
                name: global.name.clone(),
                ty: describe_type(&global.ty),
                mutable: global.mutable,
            })
            .collect();

        let functions = program
            .functions
            .iter()
            .map(|function| describe_function(program, function))
            .collect();

        let mut types = program
            .types
            .iter()
            .map(|ty| describe_type_def(program, ty))
            .collect::<Vec<_>>();
        types.extend(externals.type_descriptors());

        Self {
            module,
            globals,
            functions,
            types,
        }
    }
}

fn describe_type_def(program: &ir::Program, ty: &ir::TypeDef) -> DescriptorType {
    let method_names = ty
        .methods
        .iter()
        .filter_map(|method| program.function(*method))
        .map(|method| method.name.clone())
        .collect();

    DescriptorType {
        name: ty.name.clone(),
        kind: describe_type_kind(ty.kind).to_string(),
        origin: DescriptorOrigin::Lume,
        type_params: ty.type_params.clone(),
        fields: ty
            .fields
            .iter()
            .map(|field| DescriptorField {
                name: field.name.clone(),
                ty: describe_type(&field.ty),
                mutable: field.mutable,
            })
            .collect(),
        method_names,
    }
}

fn describe_function(program: &ir::Program, function: &ir::Function) -> DescriptorFunction {
    let owner = match function.kind {
        ir::FunctionKind::Method { owner } => program.types.get(owner.0).map(|ty| ty.name.clone()),
        _ => None,
    };

    let params = function
        .params
        .iter()
        .filter_map(|param| function.locals.get(param.0))
        .map(|local| describe_type(&local.ty))
        .collect();

    DescriptorFunction {
        name: function.name.clone(),
        owner,
        origin: DescriptorOrigin::Lume,
        params,
        return_ty: describe_type(&function.return_ty),
        block_count: function.blocks.len(),
    }
}

fn describe_type_kind(kind: TypeKind) -> &'static str {
    match kind {
        TypeKind::Annotation => "annotation",
        TypeKind::Class => "class",
        TypeKind::Record => "shape",
        TypeKind::Single => "single",
        TypeKind::Interface => "interface",
        TypeKind::Enum => "enum",
    }
}

fn describe_type(ty: &ir::Type) -> String {
    match ty {
        ir::Type::Unknown => "Unknown".to_string(),
        ir::Type::Never => "Never".to_string(),
        ir::Type::Unit => "Unit".to_string(),
        ir::Type::Bool => "Bool".to_string(),
        ir::Type::Int => "Int".to_string(),
        ir::Type::Float => "Float".to_string(),
        ir::Type::Str => "Str".to_string(),
        ir::Type::Named { name, args } if args.is_empty() => name.clone(),
        ir::Type::Named { name, args } => {
            let args = args
                .iter()
                .map(describe_type)
                .collect::<Vec<_>>()
                .join(", ");
            format!("{name}[{args}]")
        }
        ir::Type::Tuple(items) => {
            let items = items
                .iter()
                .map(describe_type)
                .collect::<Vec<_>>()
                .join(", ");
            format!("({items})")
        }
        ir::Type::Record(fields) => {
            let fields = fields
                .iter()
                .map(|field| format!("{} {}", field.name, describe_type(&field.ty)))
                .collect::<Vec<_>>()
                .join(", ");
            format!("{{{fields}}}")
        }
        ir::Type::Function { params, ret } => {
            let params = params
                .iter()
                .map(describe_type)
                .collect::<Vec<_>>()
                .join(", ");
            format!("({params}) -> {}", describe_type(ret))
        }
        ir::Type::TypeParam(name) => name.clone(),
    }
}
