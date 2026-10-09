use std::collections::{HashMap, HashSet};

use crate::{
    ast::{self, BinaryOp as AstBinaryOp, ExtensionBlock, Item, TypeDecl, TypeMember},
    core::{
        self, AssignOp, AssignmentStmt, Block, CallableBody, DestructureKind, ElseBranch,
        ElseExprBranch, Expr, FunctionDecl, MatchCaseBody, MethodDecl, Pattern, Stmt, TypeRef,
    },
    desugar,
    diagnostic::Diagnostic,
    ir,
    source::Span,
};

#[derive(Debug, Clone)]
pub struct LowerResult {
    pub program: Option<ir::Program>,
    pub core_bodies: HashMap<ir::FunctionId, core::CallableBody>,
    pub diagnostics: Vec<Diagnostic>,
}

pub fn lower_program(program: &ast::Program) -> LowerResult {
    let mut lowerer = Lowerer::new(program);
    let lowered = lowerer.lower();
    LowerResult {
        program: Some(lowered),
        core_bodies: std::mem::take(&mut lowerer.core_bodies),
        diagnostics: lowerer.diagnostics,
    }
}

#[derive(Debug, Clone)]
struct MethodWork {
    id: ir::FunctionId,
    decl: MethodDecl,
    this_local: ir::LocalId,
}

#[derive(Debug, Clone)]
struct FunctionWork {
    id: ir::FunctionId,
    decl: FunctionDecl,
}

#[derive(Debug, Clone)]
struct GlobalInit {
    id: ir::GlobalId,
    expr: Expr,
}

#[derive(Debug, Clone)]
struct FieldInitWork {
    id: ir::FunctionId,
    this_local: ir::LocalId,
    body: Block,
    span: Span,
}

struct Lowerer<'a> {
    source: &'a ast::Program,
    diagnostics: Vec<Diagnostic>,
    program: ir::Program,
    core_bodies: HashMap<ir::FunctionId, core::CallableBody>,
    type_ids: HashMap<(String, ast::TypeKind), ir::TypeId>,
    type_aliases: HashMap<String, TypeRef>,
    case_fields: HashMap<String, Vec<String>>,
    function_ids: HashMap<String, ir::FunctionId>,
    global_ids: HashMap<String, ir::GlobalId>,
    function_work: Vec<FunctionWork>,
    method_work: Vec<MethodWork>,
    field_init_work: Vec<FieldInitWork>,
    global_inits: Vec<GlobalInit>,
}

impl<'a> Lowerer<'a> {
    fn new(source: &'a ast::Program) -> Self {
        let type_aliases = source
            .items
            .iter()
            .filter_map(|item| match item {
                Item::TypeAlias(alias) => Some((alias.name.clone(), alias.target.clone())),
                _ => None,
            })
            .collect();
        Self {
            source,
            diagnostics: Vec::new(),
            program: ir::Program::new(source.module.as_ref().map(|module| module.name.clone())),
            core_bodies: HashMap::new(),
            type_ids: HashMap::new(),
            type_aliases,
            case_fields: HashMap::new(),
            function_ids: HashMap::new(),
            global_ids: HashMap::new(),
            function_work: Vec::new(),
            method_work: Vec::new(),
            field_init_work: Vec::new(),
            global_inits: Vec::new(),
        }
    }

    fn lower(&mut self) -> ir::Program {
        self.declare_top_level_items();
        self.define_items();
        self.canonicalize_nested_ir_types();
        self.derive_value_bounds();
        self.lower_top_level_functions();
        self.lower_methods();
        self.lower_field_initializers();
        self.lower_global_initializers();
        if let Some(main) = self.function_ids.get("main").copied() {
            self.program.set_entry(main);
        }
        std::mem::take(&mut self.program)
    }

    fn derive_value_bounds(&mut self) {
        let types = self.program.types.clone();
        let derived = types
            .iter()
            .map(|ty| {
                if !matches!(
                    ty.kind,
                    ast::TypeKind::Record | ast::TypeKind::Enum | ast::TypeKind::Object
                ) {
                    return None;
                }
                let self_ty = ir::Type::Named {
                    name: ty.name.clone(),
                    args: ty
                        .type_params
                        .iter()
                        .cloned()
                        .map(ir::Type::TypeParam)
                        .collect(),
                };
                let hashed = if ty.kind == ast::TypeKind::Object {
                    true
                } else {
                    ty.fields
                        .iter()
                        .chain(ty.enum_cases.iter().flat_map(|case| case.fields.iter()))
                        .all(|field| {
                            ir_type_is_hashable(&field.ty, ty, &types, &mut HashSet::new())
                        })
                };
                Some((
                    (!hashed).then(|| ir::Type::Named {
                        name: "Eq".to_string(),
                        args: vec![self_ty.clone()],
                    }),
                    hashed.then(|| ir::Type::Named {
                        name: "Hashed".to_string(),
                        args: vec![self_ty],
                    }),
                ))
            })
            .collect::<Vec<_>>();

        for (ty, bounds) in self.program.types.iter_mut().zip(derived) {
            let Some((eq, hashed)) = bounds else {
                continue;
            };
            if let Some(eq) = eq
                && !ty.with_bounds.contains(&eq)
            {
                ty.with_bounds.push(eq);
            }
            if let Some(hashed) = hashed
                && !ty.with_bounds.contains(&hashed)
            {
                ty.with_bounds.push(hashed);
            }
        }
    }

    fn canonicalize_nested_ir_types(&mut self) {
        let names = self
            .program
            .types
            .iter()
            .map(|ty| ty.name.clone())
            .collect::<HashSet<_>>();
        let aliases = self.type_aliases.clone();
        let owners = self
            .program
            .types
            .iter()
            .map(|ty| ty.name.clone())
            .collect::<Vec<_>>();

        for (index, owner) in owners.iter().enumerate() {
            let method_ids = self.program.types[index].methods.clone();
            let ty = &mut self.program.types[index];
            for bound in &mut ty.with_bounds {
                canonicalize_nested_ir_type(bound, owner, &names, &aliases);
            }
            for field in &mut ty.fields {
                canonicalize_nested_ir_type(&mut field.ty, owner, &names, &aliases);
            }
            for case in &mut ty.enum_cases {
                for field in &mut case.fields {
                    canonicalize_nested_ir_type(&mut field.ty, owner, &names, &aliases);
                }
            }
            for method_id in method_ids {
                let Some(method) = self.program.function_mut(method_id) else {
                    continue;
                };
                canonicalize_nested_ir_type(&mut method.return_ty, owner, &names, &aliases);
                for local in &mut method.locals {
                    canonicalize_nested_ir_type(&mut local.ty, owner, &names, &aliases);
                }
                for condition in &mut method.generic_conditions {
                    match condition {
                        ir::GenericCondition::Bound { subject, bound } => {
                            canonicalize_nested_ir_type(subject, owner, &names, &aliases);
                            canonicalize_nested_ir_type(bound, owner, &names, &aliases);
                        }
                        ir::GenericCondition::Equal { left, right } => {
                            canonicalize_nested_ir_type(left, owner, &names, &aliases);
                            canonicalize_nested_ir_type(right, owner, &names, &aliases);
                        }
                    }
                }
            }
        }
    }

    fn declare_top_level_items(&mut self) {
        for item in &self.source.items {
            match item {
                Item::Type(decl) => {
                    let key = (decl.name.clone(), decl.kind);
                    if self.type_ids.contains_key(&key) {
                        continue;
                    }
                    let mut ty = ir::TypeDef::new(decl.kind, decl.name.clone());
                    ty.annotations = lower_annotations(&decl.annotations);
                    ty.visibility = decl.visibility;
                    ty.type_params = decl
                        .type_params
                        .iter()
                        .map(|param| param.name.clone())
                        .collect();
                    ty.generic_conditions = lower_generic_conditions(
                        &decl.type_params,
                        &decl.type_conditions,
                        &ty.type_params,
                        &self.type_aliases,
                    );
                    ty.with_bounds = decl
                        .with_bounds
                        .iter()
                        .map(|bound| lower_type_ref_with_aliases(bound, &self.type_aliases))
                        .collect();
                    ty.span = Some(decl.span);
                    let id = self.program.add_type(ty);
                    self.type_ids.insert(key, id);
                }
                Item::Extension(block) => self.declare_builtin_extension_target(block),
                Item::Function(function) => {
                    if self.function_ids.contains_key(&function.name) {
                        continue;
                    }
                    let id = self.declare_function(
                        &function.name,
                        function.visibility,
                        &function.annotations,
                        &function.type_params,
                        &function.type_conditions,
                        &[],
                        function.return_type.as_ref(),
                        function.equals_body,
                        ir::FunctionKind::TopLevel,
                        &function.params,
                        None,
                        function.span,
                    );
                    self.function_ids.insert(function.name.clone(), id);
                    let decl = desugar::desugar_function_decl(function);
                    self.core_bodies.insert(id, decl.body.clone());
                    self.function_work.push(FunctionWork { id, decl });
                }
                Item::Statement(ast::Stmt::Binding(binding)) => {
                    for (index, local) in binding.bindings.iter().enumerate() {
                        if self.global_ids.contains_key(&local.name) || local.name == "_" {
                            continue;
                        }
                        let mut global = ir::Global::new(
                            local.name.clone(),
                            local
                                .ty
                                .as_ref()
                                .map(|ty| lower_type_ref_with_aliases(ty, &self.type_aliases))
                                .unwrap_or(ir::Type::Unknown),
                        );
                        global.visibility = binding.visibility;
                        global.mutable = local.mutable;
                        global.span = Some(local.span);
                        let id = self.program.add_global(global);
                        self.global_ids.insert(local.name.clone(), id);
                        if let Some(expr) = binding.values.get(index).cloned() {
                            self.global_inits.push(GlobalInit {
                                id,
                                expr: desugar::desugar_expr(&expr),
                            });
                        }
                    }
                }
                _ => {}
            }
        }
    }

    fn declare_builtin_extension_target(&mut self, block: &ExtensionBlock) {
        let Some(target_name) = named_type_name(&block.target) else {
            return;
        };
        if !is_builtin_extension_target(target_name) {
            return;
        }
        let key = (target_name.to_string(), ast::TypeKind::Class);
        if self.type_ids.contains_key(&key) {
            return;
        }
        let mut ty = ir::TypeDef::new(ast::TypeKind::Class, target_name);
        ty.visibility = ast::Visibility::Default;
        ty.span = Some(block.span);
        let id = self.program.add_type(ty);
        self.type_ids.insert(key, id);
    }

    fn define_items(&mut self) {
        let items = self.source.items.clone();
        for item in items {
            match item {
                Item::Type(decl) => self.define_type_decl(&decl),
                Item::Extension(block) => self.define_extension_block(&block),
                _ => {}
            }
        }
    }

    fn define_type_decl(&mut self, decl: &TypeDecl) {
        let Some(type_id) = self.type_ids.get(&(decl.name.clone(), decl.kind)).copied() else {
            return;
        };

        let mut fields = Vec::new();
        let mut methods_to_attach = Vec::new();
        let mut cases = Vec::new();
        let mut field_init_stmts = Vec::new();

        for member in &decl.members {
            match member {
                TypeMember::Field(field) => {
                    let ty = field
                        .ty
                        .as_ref()
                        .map(|ty| lower_type_ref_with_aliases(ty, &self.type_aliases))
                        .unwrap_or(ir::Type::Unknown);
                    fields.push(ir::Field {
                        annotations: lower_annotations(&field.annotations),
                        visibility: field.visibility,
                        mutable: field.mutable,
                        name: field.name.clone(),
                        ty: ty.clone(),
                        has_initializer: field.initializer.is_some(),
                        initializer: lower_field_initializer_constant(field.initializer.as_ref()),
                        span: Some(field.span),
                    });
                    if decl.kind != ast::TypeKind::Enum {
                        if let Some(initializer) = &field.initializer {
                            field_init_stmts.push(self.synthesize_field_initializer_stmt(
                                &field.name,
                                initializer,
                                field.span,
                            ));
                        }
                    }
                }
                TypeMember::Method(method) => {
                    let (id, this_local) =
                        self.declare_method_function(type_id, &decl.name, method);
                    methods_to_attach.push(id);
                    let method = desugar::desugar_method_decl(method);
                    if let Some(body) = &method.body {
                        self.core_bodies.insert(id, body.clone());
                    }
                    self.method_work.push(MethodWork {
                        id,
                        decl: method,
                        this_local,
                    });
                }
                TypeMember::Case(case) => {
                    let field_names = case
                        .fields
                        .iter()
                        .map(|field| field.name.clone())
                        .collect::<Vec<_>>();
                    self.case_fields
                        .insert(format!("{}.{}", decl.name, case.name), field_names);
                    let case_fields = case
                        .fields
                        .iter()
                        .map(|field| ir::Field {
                            annotations: lower_annotations(&field.annotations),
                            visibility: field.visibility,
                            mutable: field.mutable,
                            name: field.name.clone(),
                            ty: field
                                .ty
                                .as_ref()
                                .map(|ty| lower_type_ref_with_aliases(ty, &self.type_aliases))
                                .unwrap_or(ir::Type::Unknown),
                            has_initializer: field.initializer.is_some(),
                            initializer: lower_field_initializer_constant(
                                field.initializer.as_ref(),
                            ),
                            span: Some(field.span),
                        })
                        .collect();
                    cases.push(ir::EnumCase {
                        annotations: lower_annotations(&case.annotations),
                        kind: case.kind,
                        name: case.name.clone(),
                        fields: case_fields,
                        span: Some(case.span),
                    });
                }
            }
        }

        let field_init = (!field_init_stmts.is_empty()).then(|| {
            let (id, this_local) = self.declare_field_init_function(type_id, &decl.name, decl.span);
            let body = Block {
                statements: field_init_stmts,
                span: decl.span,
            };
            self.core_bodies
                .insert(id, CallableBody::Block(body.clone()));
            self.field_init_work.push(FieldInitWork {
                id,
                this_local,
                body,
                span: decl.span,
            });
            id
        });

        if let Some(ty) = self.program.types.get_mut(type_id.0) {
            ty.fields = fields;
            ty.field_init = field_init;
            ty.methods.extend(methods_to_attach);
            ty.enum_cases = cases;
        }
    }

    fn define_extension_block(&mut self, block: &ExtensionBlock) {
        let Some(target_name) = named_type_name(&block.target) else {
            self.add_error(
                "lower_invariant",
                "extension target should be resolved to a named type before lowering",
                block.span,
            );
            return;
        };
        let Some(type_id) = self.extension_target_type_id(target_name) else {
            self.add_error(
                "lower_invariant",
                format!(
                    "extension target '{}' should be declared before lowering methods",
                    target_name
                ),
                block.span,
            );
            return;
        };
        let mut method_ids = Vec::new();
        for method in &block.methods {
            let (id, this_local) = self.declare_method_function(type_id, target_name, method);
            method_ids.push(id);
            let method = desugar::desugar_method_decl(method);
            if let Some(body) = &method.body {
                self.core_bodies.insert(id, body.clone());
            }
            self.method_work.push(MethodWork {
                id,
                decl: method,
                this_local,
            });
        }

        if let Some(ty) = self.program.types.get_mut(type_id.0) {
            ty.methods.extend(method_ids);
        }
    }

    fn extension_target_type_id(&self, target_name: &str) -> Option<ir::TypeId> {
        self.type_ids.iter().find_map(|((name, kind), id)| {
            (name == target_name
                && matches!(
                    kind,
                    ast::TypeKind::Class
                        | ast::TypeKind::Record
                        | ast::TypeKind::Enum
                        | ast::TypeKind::Interface
                ))
            .then_some(*id)
        })
    }

    fn declare_function(
        &mut self,
        name: &str,
        visibility: ast::Visibility,
        annotations: &[ast::Annotation],
        type_params: &[ast::TypeParam],
        type_conditions: &[ast::GenericCondition],
        owner_type_params: &[String],
        return_type: Option<&TypeRef>,
        infer_return: bool,
        kind: ir::FunctionKind,
        params: &[core::Param],
        this_local: Option<(String, ir::Type)>,
        span: Span,
    ) -> ir::FunctionId {
        let type_param_names = type_params
            .iter()
            .map(|param| param.name.clone())
            .collect::<Vec<_>>();
        let available_type_params = type_param_names
            .iter()
            .cloned()
            .chain(owner_type_params.iter().cloned())
            .collect::<Vec<_>>();
        let mut function = ir::Function::new(
            name.to_string(),
            kind,
            return_type
                .map(|ty| {
                    lower_type_ref_with_type_params(ty, &available_type_params, &self.type_aliases)
                })
                .unwrap_or(if infer_return {
                    ir::Type::Unknown
                } else {
                    ir::Type::Unit
                }),
        );
        function.annotations = lower_annotations(annotations);
        function.visibility = visibility;
        function.type_params = type_param_names.clone();
        function.reified_type_params = type_params
            .iter()
            .filter(|param| param.reified)
            .map(|param| param.name.clone())
            .collect();
        function.generic_conditions = lower_generic_conditions(
            type_params,
            type_conditions,
            &available_type_params,
            &self.type_aliases,
        );
        function.span = Some(span);
        if let Some((this_name, this_ty)) = this_local {
            function.add_local(this_name, this_ty, false, ir::LocalKind::Capture);
        }
        for (index, param) in params.iter().enumerate() {
            let source_ty = param
                .ty
                .as_ref()
                .map(|ty| {
                    lower_type_ref_with_type_params(ty, &available_type_params, &self.type_aliases)
                })
                .unwrap_or(ir::Type::Unknown);
            let runtime_ty = if param.lazy {
                lazy_storage_type(source_ty)
            } else {
                source_ty
            };
            function.add_param(param.name.clone(), runtime_ty);
            function.set_param_default(
                index,
                param
                    .initializer
                    .as_ref()
                    .and_then(lower_param_default_constant),
            );
            function.set_param_variadic(index, param.variadic);
            function.set_param_lazy(index, param.lazy);
        }
        for name in function.reified_type_params.clone() {
            function.add_param(
                reified_type_param_local_name(&name),
                ir_exact_runtime_type(ir::Type::TypeParam(name)),
            );
        }
        self.program.add_function(function)
    }

    fn declare_method_function(
        &mut self,
        owner: ir::TypeId,
        owner_name: &str,
        method: &ast::MethodDecl,
    ) -> (ir::FunctionId, ir::LocalId) {
        let owner_type_params = self.program.types[owner.0].type_params.clone();
        let id = self.declare_function(
            &method.name,
            method.visibility,
            &method.annotations,
            &method.type_params,
            &method.type_conditions,
            &owner_type_params,
            method.return_type.as_ref(),
            method.equals_body && method.name != "new",
            ir::FunctionKind::Method { owner },
            &method.params,
            Some((String::from("this"), ir::Type::named(owner_name))),
            method.span,
        );
        self.program.functions[id.0].getter = method.getter;
        let this_local = self.program.functions[id.0].locals[0].id;
        (id, this_local)
    }

    fn declare_field_init_function(
        &mut self,
        owner: ir::TypeId,
        owner_name: &str,
        span: Span,
    ) -> (ir::FunctionId, ir::LocalId) {
        let id = self.declare_function(
            "__field_init",
            ast::Visibility::Private,
            &[],
            &[],
            &[],
            &self.program.types[owner.0].type_params.clone(),
            Some(&TypeRef::Named {
                name: "Unit".to_string(),
                args: Vec::new(),
                span,
            }),
            false,
            ir::FunctionKind::Method { owner },
            &[],
            Some((String::from("this"), ir::Type::named(owner_name))),
            span,
        );
        let this_local = self.program.functions[id.0].locals[0].id;
        (id, this_local)
    }

    fn lower_top_level_functions(&mut self) {
        let work = self.function_work.clone();
        for job in work {
            if self.program.function(job.id).is_none() {
                continue;
            }
            let mut lowerer = FunctionLowerer::new(
                &mut self.program,
                &mut self.core_bodies,
                job.id,
                &self.global_ids,
                &self.function_ids,
                &self.case_fields,
                &self.type_aliases,
                &mut self.diagnostics,
            );
            for (index, param) in job.decl.params.iter().enumerate() {
                if let Some(local_id) = lowerer.function().params.get(index).copied() {
                    lowerer.bind_param(param, local_id);
                }
            }
            lowerer.bind_reified_type_params();
            lowerer.lower_callable_body(&job.decl.body, job.decl.span);
        }
    }

    fn lower_methods(&mut self) {
        let work = self.method_work.clone();
        for job in work {
            if self.program.function(job.id).is_none() {
                continue;
            }
            let mut lowerer = FunctionLowerer::new(
                &mut self.program,
                &mut self.core_bodies,
                job.id,
                &self.global_ids,
                &self.function_ids,
                &self.case_fields,
                &self.type_aliases,
                &mut self.diagnostics,
            );
            lowerer.bind_existing("this", job.this_local);
            for (index, param) in job.decl.params.iter().enumerate() {
                if let Some(local_id) = lowerer.function().params.get(index).copied() {
                    lowerer.bind_param(param, local_id);
                }
            }
            lowerer.bind_reified_type_params();
            if let Some(body) = &job.decl.body {
                lowerer.lower_callable_body(body, job.decl.span);
            } else if let Some(block) = lowerer.current_block_mut() {
                block.set_terminator(ir::Terminator::ret(Some(ir::Operand::Const(
                    ir::Constant::Unit,
                ))));
            }
        }
    }

    fn lower_global_initializers(&mut self) {
        if self.global_inits.is_empty() {
            return;
        }

        let mut init = ir::Function::new(
            "__globals_init",
            ir::FunctionKind::Synthetic,
            ir::Type::Unit,
        );
        init.visibility = ast::Visibility::Private;
        let init_id = self.program.add_function(init);
        self.program.set_global_init(init_id);

        let jobs = self.global_inits.clone();
        let mut lowerer = FunctionLowerer::new(
            &mut self.program,
            &mut self.core_bodies,
            init_id,
            &self.global_ids,
            &self.function_ids,
            &self.case_fields,
            &self.type_aliases,
            &mut self.diagnostics,
        );
        for job in jobs {
            let expected = lowerer
                .program
                .globals
                .get(job.id.0)
                .map(|global| global.ty.clone())
                .unwrap_or(ir::Type::Unknown);
            let value = lowerer.lower_expr_with_expected(&job.expr, Some(&expected));
            lowerer.push_statement(ir::Statement {
                span: Some(job.expr.span()),
                kind: ir::StatementKind::Assign {
                    target: ir::Place::Global(job.id),
                    value: ir::RValue::Use(value),
                },
            });
        }
        if let Some(block) = lowerer.current_block_mut() {
            block.set_terminator(ir::Terminator::ret(Some(ir::Operand::Const(
                ir::Constant::Unit,
            ))));
        }
    }

    fn lower_field_initializers(&mut self) {
        let work = self.field_init_work.clone();
        for job in work {
            if self.program.function(job.id).is_none() {
                continue;
            }
            let mut lowerer = FunctionLowerer::new(
                &mut self.program,
                &mut self.core_bodies,
                job.id,
                &self.global_ids,
                &self.function_ids,
                &self.case_fields,
                &self.type_aliases,
                &mut self.diagnostics,
            );
            lowerer.bind_existing("this", job.this_local);
            lowerer.lower_callable_body(&CallableBody::Block(job.body), job.span);
        }
    }

    fn synthesize_field_initializer_stmt(
        &self,
        field_name: &str,
        initializer: &ast::Expr,
        span: Span,
    ) -> Stmt {
        Stmt::Assignment(AssignmentStmt {
            targets: vec![Expr::Member {
                receiver: Box::new(Expr::Identifier {
                    name: "this".to_string(),
                    span,
                }),
                name: field_name.to_string(),
                span,
            }],
            operator: AssignOp::Reassign,
            values: vec![desugar::desugar_expr(initializer)],
            span,
        })
    }

    fn add_error(&mut self, code: &'static str, message: impl Into<String>, span: Span) {
        self.diagnostics
            .push(Diagnostic::error(code, message, span));
    }
}

struct FunctionLowerer<'a> {
    program: &'a mut ir::Program,
    core_bodies: &'a mut HashMap<ir::FunctionId, core::CallableBody>,
    function_id: ir::FunctionId,
    diagnostics: &'a mut Vec<Diagnostic>,
    globals: &'a HashMap<String, ir::GlobalId>,
    functions: &'a HashMap<String, ir::FunctionId>,
    case_fields: &'a HashMap<String, Vec<String>>,
    type_aliases: &'a HashMap<String, TypeRef>,
    scopes: Vec<HashMap<String, ir::LocalId>>,
    capture_sources: HashMap<String, CaptureSource>,
    capture_locals: HashMap<String, ir::LocalId>,
    closure_captures: Vec<ir::Operand>,
    lazy_values: HashMap<String, ir::Type>,
    this_local: Option<ir::LocalId>,
    loop_exits: Vec<ir::BlockId>,
    loop_continues: Vec<ir::BlockId>,
    current_block: Option<ir::BlockId>,
}

#[derive(Debug, Clone)]
struct CaptureSource {
    operand: ir::Operand,
    ty: ir::Type,
    lazy_value_ty: Option<ir::Type>,
}

#[derive(Debug, Clone)]
struct IrTypeNarrowing {
    name: String,
    ty: ir::Type,
    span: Span,
}

#[derive(Debug, Clone)]
struct ExpectedArgSpec {
    name: Option<String>,
    ty: ir::Type,
    lazy: bool,
    variadic: bool,
    default: Option<ir::Constant>,
}

#[derive(Debug, Clone, PartialEq)]
enum LiftedIrFamily {
    Option,
    Result { error: ir::Type },
    Either { left: ir::Type },
}

#[derive(Debug, Clone)]
struct PendingBinding {
    name: String,
    ty: ir::Type,
    source: PendingBindingSource,
}

#[derive(Debug, Clone)]
struct AppliedBinding {
    name: String,
    ty: ir::Type,
    local: ir::LocalId,
}

#[derive(Debug, Clone)]
enum PendingBindingSource {
    Operand(ir::Operand),
    RValue(ir::RValue),
}

#[derive(Debug, Clone)]
struct PatternPlan {
    condition: ir::Operand,
    bindings: Vec<PendingBinding>,
}

#[derive(Debug, Clone)]
enum ConstructorPatternKind {
    EnumCase {
        case_name: String,
        field_names: Vec<String>,
    },
    TypeDestructure {
        ty: ir::Type,
        field_names: Vec<String>,
    },
    ObjectSingleton {
        ty: ir::Type,
    },
}

impl PatternPlan {
    fn always_true() -> Self {
        Self {
            condition: ir::Operand::Const(ir::Constant::Bool(true)),
            bindings: Vec::new(),
        }
    }

    fn always_false() -> Self {
        Self {
            condition: ir::Operand::Const(ir::Constant::Bool(false)),
            bindings: Vec::new(),
        }
    }
}

impl<'a> FunctionLowerer<'a> {
    fn new(
        program: &'a mut ir::Program,
        core_bodies: &'a mut HashMap<ir::FunctionId, core::CallableBody>,
        function_id: ir::FunctionId,
        globals: &'a HashMap<String, ir::GlobalId>,
        functions: &'a HashMap<String, ir::FunctionId>,
        case_fields: &'a HashMap<String, Vec<String>>,
        type_aliases: &'a HashMap<String, TypeRef>,
        diagnostics: &'a mut Vec<Diagnostic>,
    ) -> Self {
        let entry = program
            .function(function_id)
            .map(|function| function.entry)
            .unwrap_or(ir::BlockId(0));
        let mut this = Self {
            program,
            core_bodies,
            function_id,
            diagnostics,
            globals,
            functions,
            case_fields,
            type_aliases,
            scopes: vec![HashMap::new()],
            capture_sources: HashMap::new(),
            capture_locals: HashMap::new(),
            closure_captures: Vec::new(),
            lazy_values: HashMap::new(),
            this_local: None,
            loop_exits: Vec::new(),
            loop_continues: Vec::new(),
            current_block: Some(entry),
        };
        if let Some(first) = this.function().locals.first() {
            if first.name == "this" {
                this.this_local = Some(first.id);
            }
        }
        this
    }

    fn with_capture_sources(mut self, capture_sources: HashMap<String, CaptureSource>) -> Self {
        self.capture_sources = capture_sources;
        self
    }

    fn finish_closure_captures(self) -> Vec<ir::Operand> {
        self.closure_captures
    }

    fn function(&self) -> &ir::Function {
        self.program
            .function(self.function_id)
            .expect("active lowered function")
    }

    fn function_mut(&mut self) -> &mut ir::Function {
        self.program
            .function_mut(self.function_id)
            .expect("active lowered function")
    }

    fn lexical_owner_name(&self) -> Option<String> {
        if let ir::FunctionKind::Method { owner } = self.function().kind {
            return self.program.types.get(owner.0).map(|ty| ty.name.clone());
        }
        self.capture_sources
            .get("this")
            .and_then(|source| match &source.ty {
                ir::Type::Named { name, .. } => Some(name.clone()),
                _ => None,
            })
    }

    fn canonical_declared_type_path(&self, path: &[String]) -> Option<String> {
        let joined = path.join(".");
        if declared_type_exists(self.program, &joined) {
            return Some(joined);
        }
        if path.len() != 1 {
            return None;
        }
        let owner = self.lexical_owner_name()?;
        let names = self
            .program
            .types
            .iter()
            .map(|ty| ty.name.clone())
            .collect::<HashSet<_>>();
        lexical_nested_ir_name(&owner, &path[0], &names)
    }

    fn canonical_enum_case_path(&self, path: &[String]) -> Vec<String> {
        let Some((case_name, owner_path)) = path.split_last() else {
            return Vec::new();
        };
        let Some(owner_name) = self.canonical_declared_type_path(owner_path) else {
            return path.to_vec();
        };
        let is_case = self.program.types.iter().any(|ty| {
            ty.kind == ast::TypeKind::Enum
                && ty.name == owner_name
                && ty.enum_cases.iter().any(|case| case.name == *case_name)
        });
        if is_case {
            vec![owner_name, case_name.clone()]
        } else {
            path.to_vec()
        }
    }

    fn lower_type_ref(&self, reference: &TypeRef) -> ir::Type {
        let mut ty = lower_type_ref_with_aliases(reference, self.type_aliases);
        if let Some(owner) = self.lexical_owner_name() {
            let names = self
                .program
                .types
                .iter()
                .map(|ty| ty.name.clone())
                .collect::<HashSet<_>>();
            canonicalize_nested_ir_type(&mut ty, &owner, &names, self.type_aliases);
        }
        ty
    }

    fn add_local(
        &mut self,
        name: impl Into<String>,
        ty: ir::Type,
        mutable: bool,
        kind: ir::LocalKind,
    ) -> ir::LocalId {
        self.function_mut().add_local(name, ty, mutable, kind)
    }

    fn add_capture(&mut self, name: impl Into<String>, ty: ir::Type) -> ir::LocalId {
        self.function_mut().add_capture(name, ty)
    }

    fn add_temp(&mut self, ty: ir::Type) -> ir::LocalId {
        self.function_mut().add_temp(ty)
    }

    fn add_block(&mut self) -> ir::BlockId {
        self.function_mut().add_block()
    }

    fn bind_existing(&mut self, name: &str, local: ir::LocalId) {
        self.current_scope().insert(name.to_string(), local);
        if name == "this" {
            self.this_local = Some(local);
        }
    }

    fn bind_param(&mut self, param: &core::Param, local: ir::LocalId) {
        if param.name == "_" {
            return;
        }
        self.bind_existing(&param.name, local);
        if param.lazy {
            let value_ty = self
                .function()
                .locals
                .get(local.0)
                .and_then(|local| lazy_value_type(&local.ty))
                .unwrap_or(ir::Type::Unknown);
            self.lazy_values.insert(param.name.clone(), value_ty);
        }
    }

    fn bind_reified_type_params(&mut self) {
        let names = self.function().reified_type_params.clone();
        for name in names {
            let local_name = reified_type_param_local_name(&name);
            let Some(local_id) = self.function().params.iter().copied().find(|param| {
                self.function()
                    .locals
                    .get(param.0)
                    .is_some_and(|local| local.name == local_name)
            }) else {
                continue;
            };
            self.bind_existing(&local_name, local_id);
        }
    }

    fn lower_callable_body(&mut self, body: &CallableBody, span: Span) {
        let return_ty = self.function().return_ty.clone();
        let result = match body {
            CallableBody::Expr(expr) => Some(self.lower_expr_with_expected(expr, Some(&return_ty))),
            CallableBody::Block(block) => {
                self.lower_block_value_with_expected(block, Some(&return_ty))
            }
        };
        if matches!(return_ty, ir::Type::Unknown)
            && let Some(inferred) = result.as_ref().and_then(|value| self.operand_type(value))
            && !matches!(inferred, ir::Type::Unknown)
        {
            self.function_mut().return_ty = inferred;
        }
        if let Some(block) = self.current_block_mut() {
            block.set_terminator(ir::Terminator {
                span: Some(span),
                kind: ir::TerminatorKind::Return(result),
            });
        }
    }

    fn lower_block_value(&mut self, block: &Block) -> Option<ir::Operand> {
        self.lower_block_value_with_expected(block, None)
    }

    fn lower_block_value_with_expected(
        &mut self,
        block: &Block,
        expected: Option<&ir::Type>,
    ) -> Option<ir::Operand> {
        self.push_scope();
        let mut tail = None;
        for (index, stmt) in block.statements.iter().enumerate() {
            let is_last = index + 1 == block.statements.len();
            if is_last {
                match stmt {
                    Stmt::Expr(expr_stmt) => {
                        tail = Some(self.lower_expr_with_expected(&expr_stmt.expr, expected));
                        break;
                    }
                    Stmt::If(if_stmt) => {
                        if let Some(value) = self.lower_if_stmt_tail_value(if_stmt, expected) {
                            tail = Some(value);
                            break;
                        }
                    }
                    Stmt::Match(match_stmt) => {
                        tail = Some(self.lower_match_expr_with_expected(
                            &match_stmt.value,
                            &match_stmt.cases,
                            match_stmt.span,
                            expected,
                        ));
                        break;
                    }
                    _ => {}
                }
            }
            self.lower_stmt(stmt);
            if self.current_block.is_none() {
                break;
            }
        }
        self.pop_scope();
        tail
    }

    fn lower_block_statements(&mut self, block: &Block) {
        self.push_scope();
        for stmt in &block.statements {
            self.lower_stmt(stmt);
            if self.current_block.is_none() {
                break;
            }
        }
        self.pop_scope();
    }

    fn lower_stmt(&mut self, stmt: &Stmt) {
        if self.current_block.is_none() {
            return;
        }
        match stmt {
            Stmt::Binding(binding) => {
                let destructure_single_value =
                    binding.destructure.is_some() && binding.values.len() == 1;
                let source_value =
                    destructure_single_value.then(|| self.lower_expr(&binding.values[0]));
                let destructure_fields =
                    matches!(binding.destructure, Some(DestructureKind::Record)).then(|| {
                        self.destructure_field_names(&binding.values[0], &binding.bindings)
                    });
                for (index, local) in binding.bindings.iter().enumerate() {
                    if local.name == "_" {
                        if let Some(value) = if destructure_single_value {
                            source_value.clone().map(|base| {
                                self.emit_temp_from_rvalue(
                                    ir::RValue::Field {
                                        base,
                                        name: destructure_fields
                                            .as_ref()
                                            .and_then(|fields| fields.get(index).cloned())
                                            .unwrap_or_else(|| format!("_{}", index + 1)),
                                    },
                                    ir::Type::Unknown,
                                    Some(local.span),
                                )
                            })
                        } else {
                            binding.values.get(index).map(|expr| self.lower_expr(expr))
                        } {
                            self.push_statement(ir::Statement {
                                span: Some(local.span),
                                kind: ir::StatementKind::Eval {
                                    value: ir::RValue::Use(value),
                                },
                            });
                        }
                        continue;
                    }
                    let ty = local
                        .ty
                        .as_ref()
                        .map(|ty| self.lower_type_ref(ty))
                        .unwrap_or_else(|| {
                            if destructure_single_value {
                                ir::Type::Unknown
                            } else {
                                binding
                                    .values
                                    .get(index)
                                    .map(|expr| {
                                        inferred_storage_type(
                                            self.infer_expr_type_against_with_overrides(
                                                expr,
                                                &ir::Type::Unknown,
                                                &[],
                                            ),
                                        )
                                    })
                                    .unwrap_or(ir::Type::Unknown)
                            }
                        });
                    let local_id = self.add_local(
                        local.name.clone(),
                        ty.clone(),
                        local.mutable,
                        ir::LocalKind::Binding,
                    );
                    self.current_scope().insert(local.name.clone(), local_id);
                    if let Some(value) = if destructure_single_value {
                        source_value.clone().map(|base| {
                            self.emit_temp_from_rvalue(
                                ir::RValue::Field {
                                    base,
                                    name: destructure_fields
                                        .as_ref()
                                        .and_then(|fields| fields.get(index).cloned())
                                        .unwrap_or_else(|| format!("_{}", index + 1)),
                                },
                                ir::Type::Unknown,
                                Some(local.span),
                            )
                        })
                    } else {
                        binding
                            .values
                            .get(index)
                            .map(|expr| self.lower_expr_with_expected(expr, Some(&ty)))
                    } {
                        self.push_statement(ir::Statement {
                            span: Some(local.span),
                            kind: ir::StatementKind::Assign {
                                target: ir::Place::Local(local_id),
                                value: ir::RValue::Use(value),
                            },
                        });
                    }
                }
            }
            Stmt::PatternBinding(stmt) => self.lower_pattern_binding_stmt(stmt),
            Stmt::Assignment(assignment) => {
                for (target_expr, value_expr) in
                    assignment.targets.iter().zip(assignment.values.iter())
                {
                    let expected = self.infer_assignment_target_type(target_expr);
                    let Some(target) = self.lower_place(target_expr) else {
                        continue;
                    };
                    let value =
                        if matches!(assignment.operator, AssignOp::Assign | AssignOp::Reassign) {
                            ir::RValue::Use(
                                self.lower_expr_with_expected(value_expr, Some(&expected)),
                            )
                        } else {
                            let Some(op) = map_assign_op(assignment.operator) else {
                                self.invariant(
                                    "assignment operator should map before lowering",
                                    assignment.span,
                                );
                                continue;
                            };
                            let current = ir::Operand::Copy(Box::new(target.clone()));
                            ir::RValue::Binary {
                                op,
                                left: current,
                                right: self.lower_expr(value_expr),
                            }
                        };
                    self.push_statement(ir::Statement {
                        span: Some(assignment.span),
                        kind: ir::StatementKind::Assign { target, value },
                    });
                }
            }
            Stmt::Defer(stmt) => self.lower_defer_stmt(stmt),
            Stmt::LetElse(stmt) => self.lower_let_else_stmt(stmt),
            Stmt::If(stmt) => self.lower_if_stmt(stmt),
            Stmt::While(stmt) => self.lower_while_stmt(stmt),
            Stmt::For(stmt) => self.lower_for_stmt(stmt),
            Stmt::Return(ret) => {
                let return_ty = self.function().return_ty.clone();
                let value = ret
                    .value
                    .as_ref()
                    .map(|expr| self.lower_expr_with_expected(expr, Some(&return_ty)));
                self.terminate(ir::Terminator {
                    span: Some(ret.span),
                    kind: ir::TerminatorKind::Return(value),
                });
            }
            Stmt::Break(stmt) => {
                if let Some(exit) = self.loop_exits.last().copied() {
                    self.terminate(ir::Terminator {
                        span: Some(stmt.span),
                        kind: ir::TerminatorKind::Goto(exit),
                    });
                } else {
                    self.invariant("break should be rejected before lowering", stmt.span);
                }
            }
            Stmt::Continue(stmt) => {
                if let Some(target) = self.loop_continues.last().copied() {
                    self.terminate(ir::Terminator {
                        span: Some(stmt.span),
                        kind: ir::TerminatorKind::Goto(target),
                    });
                } else {
                    self.invariant("continue should be rejected before lowering", stmt.span);
                }
            }
            Stmt::Expr(expr) => {
                let value = self.lower_expr(&expr.expr);
                self.push_statement(ir::Statement {
                    span: Some(expr.span),
                    kind: ir::StatementKind::Eval {
                        value: ir::RValue::Use(value),
                    },
                });
            }
            Stmt::Match(stmt) => self.lower_match_stmt(stmt),
            Stmt::LocalFunction(function) => self.lower_local_function_stmt(function),
        }
    }

    fn lower_local_function_stmt(&mut self, function: &FunctionDecl) {
        let ty = ir::Type::Function {
            params: function
                .params
                .iter()
                .map(|param| {
                    param
                        .ty
                        .as_ref()
                        .map(|ty| self.lower_type_ref(ty))
                        .unwrap_or(ir::Type::Unknown)
                })
                .collect(),
            ret: Box::new(
                function
                    .return_type
                    .as_ref()
                    .map(|ty| self.lower_type_ref(ty))
                    .unwrap_or(if function.equals_body {
                        ir::Type::Unknown
                    } else {
                        ir::Type::Unit
                    }),
            ),
        };
        let local_id = self.add_local(function.name.clone(), ty, false, ir::LocalKind::Binding);
        self.current_scope().insert(function.name.clone(), local_id);

        let closure = self.lower_nested_function_decl(function);
        if let ir::RValue::Closure {
            function: function_id,
            ..
        } = &closure
            && let Some(inferred) = self
                .program
                .function(*function_id)
                .map(|nested| nested.return_ty.clone())
            && let Some(local) = self.function_mut().locals.get_mut(local_id.0)
            && let ir::Type::Function { ret, .. } = &mut local.ty
            && matches!(ret.as_ref(), ir::Type::Unknown)
        {
            **ret = inferred;
        }
        self.push_statement(ir::Statement {
            span: Some(function.span),
            kind: ir::StatementKind::Assign {
                target: ir::Place::Local(local_id),
                value: closure,
            },
        });
    }

    fn lower_defer_stmt(&mut self, stmt: &core::DeferStmt) {
        let body = match &stmt.action {
            core::DeferAction::Call(expr) => CallableBody::Expr(expr.clone()),
            core::DeferAction::Block(block) => CallableBody::Block(block.clone()),
        };
        let closure = self.lower_callable_closure("defer", &[], None, Some(&body), stmt.span);
        self.push_statement(ir::Statement {
            span: Some(stmt.span),
            kind: ir::StatementKind::Defer { value: closure },
        });
    }

    fn lower_nested_function_decl(&mut self, function: &FunctionDecl) -> ir::RValue {
        let mut nested = ir::Function::new(
            function.name.clone(),
            ir::FunctionKind::Local {
                parent: self.function_id,
            },
            function
                .return_type
                .as_ref()
                .map(|ty| self.lower_type_ref(ty))
                .unwrap_or(if function.equals_body {
                    ir::Type::Unknown
                } else {
                    ir::Type::Unit
                }),
        );
        nested.span = Some(function.span);
        for (index, param) in function.params.iter().enumerate() {
            let source_ty = param
                .ty
                .as_ref()
                .map(|ty| self.lower_type_ref(ty))
                .unwrap_or(ir::Type::Unknown);
            let runtime_ty = if param.lazy {
                lazy_storage_type(source_ty)
            } else {
                source_ty
            };
            nested.add_param(param.name.clone(), runtime_ty);
            nested.set_param_variadic(index, param.variadic);
            nested.set_param_lazy(index, param.lazy);
        }
        let function_id = self.program.add_function(nested);
        let capture_sources = self.visible_capture_sources(Some(&function.name));
        let captures = {
            let mut lowerer = FunctionLowerer::new(
                self.program,
                self.core_bodies,
                function_id,
                self.globals,
                self.functions,
                self.case_fields,
                self.type_aliases,
                self.diagnostics,
            )
            .with_capture_sources(capture_sources);
            for (index, param) in function.params.iter().enumerate() {
                if let Some(local_id) = lowerer.function().params.get(index).copied() {
                    lowerer.bind_param(param, local_id);
                }
            }
            lowerer.lower_callable_body(&function.body, function.span);
            lowerer.finish_closure_captures()
        };
        ir::RValue::Closure {
            function: function_id,
            captures,
        }
    }

    fn lower_lambda_rvalue(
        &mut self,
        params: &[core::LambdaParam],
        body: &Expr,
        span: Span,
        expected_params: Option<&[ir::Type]>,
        expected_return: Option<ir::Type>,
    ) -> ir::RValue {
        let nested_name = format!(
            "lambda${}${}",
            self.function_id.0,
            self.function().blocks.len()
        );
        let mut nested = ir::Function::new(
            nested_name,
            ir::FunctionKind::Lambda,
            expected_return.unwrap_or(ir::Type::Unknown),
        );
        nested.span = Some(span);
        for (index, param) in params.iter().enumerate() {
            nested.add_param(
                lower_lambda_param_name(param, index),
                lower_lambda_param_type(
                    param,
                    expected_params.and_then(|params| params.get(index)),
                    self.type_aliases,
                ),
            );
        }
        let function_id = self.program.add_function(nested);
        self.core_bodies
            .insert(function_id, CallableBody::Expr(body.clone()));
        let capture_sources = self.visible_capture_sources(None);
        let captures = {
            let mut lowerer = FunctionLowerer::new(
                self.program,
                self.core_bodies,
                function_id,
                self.globals,
                self.functions,
                self.case_fields,
                self.type_aliases,
                self.diagnostics,
            )
            .with_capture_sources(capture_sources);
            for (index, param) in params.iter().enumerate() {
                if let Some(local_id) = lowerer.function().params.get(index).copied() {
                    if let Some(destructure) = &param.destructure {
                        let value = ir::Operand::Copy(Box::new(ir::Place::Local(local_id)));
                        lowerer.bind_lambda_destructure_param(destructure, value);
                    } else if param.name != "_" {
                        lowerer.bind_existing(&param.name, local_id);
                    }
                }
            }
            lowerer.lower_callable_body(&CallableBody::Expr(body.clone()), span);
            lowerer.finish_closure_captures()
        };
        ir::RValue::Closure {
            function: function_id,
            captures,
        }
    }

    fn lower_callable_reference_rvalue(
        &mut self,
        reference: &Expr,
        expected: &ir::Type,
    ) -> ir::RValue {
        let ir::Type::Function { params, ret } = expected else {
            return ir::RValue::Use(ir::Operand::Const(ir::Constant::Unit));
        };
        let nested_name = format!(
            "methodref${}${}",
            self.function_id.0,
            self.function().blocks.len()
        );
        let mut nested =
            ir::Function::new(nested_name, ir::FunctionKind::Lambda, ret.as_ref().clone());
        let span = reference.span();
        nested.span = Some(span);
        let param_names = params
            .iter()
            .enumerate()
            .map(|(index, ty)| {
                let name = format!("__method_ref_arg{index}");
                nested.add_param(name.clone(), ty.clone());
                name
            })
            .collect::<Vec<_>>();
        let function_id = self.program.add_function(nested);
        let body = callable_reference_call_expr(reference, &param_names, span);
        let capture_sources = self.visible_capture_sources(None);
        let captures = {
            let mut lowerer = FunctionLowerer::new(
                self.program,
                self.core_bodies,
                function_id,
                self.globals,
                self.functions,
                self.case_fields,
                self.type_aliases,
                self.diagnostics,
            )
            .with_capture_sources(capture_sources);
            for (index, name) in param_names.iter().enumerate() {
                if let Some(local_id) = lowerer.function().params.get(index).copied() {
                    lowerer.bind_existing(name, local_id);
                }
            }
            lowerer.lower_callable_body(&CallableBody::Expr(body), span);
            lowerer.finish_closure_captures()
        };
        ir::RValue::Closure {
            function: function_id,
            captures,
        }
    }

    fn lower_callable_closure(
        &mut self,
        name: &str,
        params: &[core::Param],
        return_type: Option<&TypeRef>,
        body: Option<&CallableBody>,
        span: Span,
    ) -> ir::RValue {
        let nested_name = format!("anon${}${name}", self.function_id.0);
        let mut nested = ir::Function::new(
            nested_name,
            ir::FunctionKind::Lambda,
            return_type
                .map(|ty| self.lower_type_ref(ty))
                .unwrap_or(ir::Type::Unknown),
        );
        nested.span = Some(span);
        for (index, param) in params.iter().enumerate() {
            let source_ty = param
                .ty
                .as_ref()
                .map(|ty| self.lower_type_ref(ty))
                .unwrap_or(ir::Type::Unknown);
            let runtime_ty = if param.lazy {
                lazy_storage_type(source_ty)
            } else {
                source_ty
            };
            nested.add_param(param.name.clone(), runtime_ty);
            nested.set_param_variadic(index, param.variadic);
            nested.set_param_lazy(index, param.lazy);
        }
        let function_id = self.program.add_function(nested);
        if let Some(body) = body {
            self.core_bodies.insert(function_id, body.clone());
        }
        let capture_sources = self.visible_capture_sources(None);
        let captures = {
            let mut lowerer = FunctionLowerer::new(
                self.program,
                self.core_bodies,
                function_id,
                self.globals,
                self.functions,
                self.case_fields,
                self.type_aliases,
                self.diagnostics,
            )
            .with_capture_sources(capture_sources);
            for (index, param) in params.iter().enumerate() {
                if let Some(local_id) = lowerer.function().params.get(index).copied() {
                    lowerer.bind_param(param, local_id);
                }
            }
            if let Some(body) = body {
                lowerer.lower_callable_body(body, span);
            } else if let Some(block) = lowerer.current_block_mut() {
                block.set_terminator(ir::Terminator::ret(Some(ir::Operand::Const(
                    ir::Constant::Unit,
                ))));
            }
            lowerer.finish_closure_captures()
        };
        ir::RValue::Closure {
            function: function_id,
            captures,
        }
    }

    fn lower_anonymous_object_rvalue(
        &mut self,
        kind: ast::TypeKind,
        interfaces: &[TypeRef],
        fields: &[core::FieldDecl],
        methods: &[MethodDecl],
        span: Span,
    ) -> ir::RValue {
        let type_name = crate::source::anonymous_object_type_name(span);
        let mut ty = ir::TypeDef::new(kind, type_name.clone());
        ty.visibility = ast::Visibility::Private;
        ty.span = Some(span);
        ty.with_bounds = interfaces
            .iter()
            .map(|interface| self.lower_type_ref(interface))
            .collect();
        ty.fields = fields
            .iter()
            .map(|field| ir::Field {
                annotations: lower_annotations(&field.annotations),
                visibility: field.visibility,
                mutable: false,
                name: field.name.clone(),
                ty: field
                    .ty
                    .as_ref()
                    .map(|ty| self.lower_type_ref(ty))
                    .or_else(|| {
                        field
                            .initializer
                            .as_ref()
                            .map(|value| self.infer_expr_type(value))
                    })
                    .unwrap_or(ir::Type::Unknown),
                has_initializer: true,
                initializer: None,
                span: Some(field.span),
            })
            .collect();
        let owner = self.program.add_type(ty);

        let mut method_ids = Vec::new();
        for field in fields {
            let return_ty = self.program.types[owner.0]
                .fields
                .iter()
                .find(|candidate| candidate.name == field.name)
                .map(|candidate| candidate.ty.clone())
                .unwrap_or(ir::Type::Unknown);
            let mut getter = ir::Function::new(
                field.name.clone(),
                ir::FunctionKind::Method { owner },
                return_ty,
            );
            getter.visibility = field.visibility;
            getter.span = Some(field.span);
            getter.add_local(
                "this",
                ir::Type::named(type_name.clone()),
                false,
                ir::LocalKind::Capture,
            );
            method_ids.push(self.program.add_function(getter));
        }

        let mut declared_methods = Vec::new();
        for method in methods {
            let mut function = ir::Function::new(
                method.name.clone(),
                ir::FunctionKind::Method { owner },
                method
                    .return_type
                    .as_ref()
                    .map(|ty| self.lower_type_ref(ty))
                    .unwrap_or(ir::Type::Unknown),
            );
            function.annotations = lower_annotations(&method.annotations);
            function.visibility = method.visibility;
            function.getter = method.getter;
            function.type_params = method
                .type_params
                .iter()
                .map(|param| param.name.clone())
                .collect();
            function.reified_type_params = method
                .type_params
                .iter()
                .filter(|param| param.reified)
                .map(|param| param.name.clone())
                .collect();
            function.span = Some(method.span);
            function.add_local(
                "this",
                ir::Type::named(type_name.clone()),
                false,
                ir::LocalKind::Capture,
            );
            for (index, param) in method.params.iter().enumerate() {
                let source_ty = param
                    .ty
                    .as_ref()
                    .map(|ty| self.lower_type_ref(ty))
                    .unwrap_or(ir::Type::Unknown);
                let runtime_ty = if param.lazy {
                    lazy_storage_type(source_ty)
                } else {
                    source_ty
                };
                function.add_param(param.name.clone(), runtime_ty);
                function.set_param_variadic(index, param.variadic);
                function.set_param_lazy(index, param.lazy);
            }
            let id = self.program.add_function(function);
            method_ids.push(id);
            declared_methods.push((method, id));
        }
        self.program.types[owner.0].methods = method_ids;

        self.push_scope();
        let mut lowered_fields = Vec::new();
        for field in fields {
            let field_ty = self.program.types[owner.0]
                .fields
                .iter()
                .find(|candidate| candidate.name == field.name)
                .map(|candidate| candidate.ty.clone())
                .unwrap_or(ir::Type::Unknown);
            let value = field
                .initializer
                .as_ref()
                .map(|initializer| self.lower_expr_with_expected(initializer, Some(&field_ty)))
                .unwrap_or(ir::Operand::Const(ir::Constant::Unit));
            let local = self.add_local(field.name.clone(), field_ty, false, ir::LocalKind::Binding);
            self.push_statement(ir::Statement {
                span: Some(field.span),
                kind: ir::StatementKind::Assign {
                    target: ir::Place::Local(local),
                    value: ir::RValue::Use(value),
                },
            });
            self.bind_existing(&field.name, local);
            lowered_fields.push(ir::NamedOperand {
                name: field.name.clone(),
                value: ir::Operand::Copy(Box::new(ir::Place::Local(local))),
            });
        }
        self.pop_scope();

        let capture_sources = self.visible_capture_sources(Some("this"));
        let mut lowered_methods = Vec::new();
        for (method, function_id) in declared_methods {
            if let Some(body) = &method.body {
                self.core_bodies.insert(function_id, body.clone());
            }
            let captures = {
                let mut lowerer = FunctionLowerer::new(
                    self.program,
                    self.core_bodies,
                    function_id,
                    self.globals,
                    self.functions,
                    self.case_fields,
                    self.type_aliases,
                    self.diagnostics,
                )
                .with_capture_sources(capture_sources.clone());
                if let Some(this_local) = lowerer.this_local {
                    lowerer.bind_existing("this", this_local);
                }
                for (index, param) in method.params.iter().enumerate() {
                    if let Some(local_id) = lowerer.function().params.get(index).copied() {
                        lowerer.bind_param(param, local_id);
                    }
                }
                lowerer.bind_reified_type_params();
                if let Some(body) = &method.body {
                    lowerer.lower_callable_body(body, method.span);
                }
                lowerer.finish_closure_captures()
            };
            lowered_methods.push(ir::AnonymousObjectMethod {
                name: method.name.clone(),
                function: function_id,
                captures,
            });
        }

        ir::RValue::AnonymousObject {
            ty: ir::Type::named(type_name),
            fields: lowered_fields,
            methods: lowered_methods,
        }
    }

    fn visible_capture_sources(
        &mut self,
        excluded_name: Option<&str>,
    ) -> HashMap<String, CaptureSource> {
        // A nested closure can be the first place an enclosing value is used.
        // Forward inherited captures through this function's own frame so the
        // child never refers directly to a grandparent function's local.
        let inherited_names = self
            .capture_sources
            .keys()
            .filter(|name| {
                name.as_str() != "_"
                    && !excluded_name.is_some_and(|excluded| excluded == name.as_str())
            })
            .cloned()
            .collect::<Vec<_>>();
        for name in inherited_names {
            self.capture_local(&name);
        }

        let mut sources = HashMap::new();
        for scope in &self.scopes {
            for (name, local) in scope {
                if name == "_" || excluded_name.is_some_and(|excluded| excluded == name) {
                    continue;
                }
                let ty = self
                    .function()
                    .locals
                    .get(local.0)
                    .map(|local| local.ty.clone())
                    .unwrap_or(ir::Type::Unknown);
                let lazy_value_ty = self.lazy_values.get(name).cloned();
                sources.insert(
                    name.clone(),
                    CaptureSource {
                        operand: ir::Operand::Copy(Box::new(ir::Place::Local(*local))),
                        ty,
                        lazy_value_ty,
                    },
                );
            }
        }
        if let Some(this_local) = self.this_local {
            sources
                .entry("this".to_string())
                .or_insert_with(|| CaptureSource {
                    operand: ir::Operand::Copy(Box::new(ir::Place::Local(this_local))),
                    ty: self
                        .function()
                        .locals
                        .get(this_local.0)
                        .map(|local| local.ty.clone())
                        .unwrap_or(ir::Type::Unknown),
                    lazy_value_ty: None,
                });
        }
        sources
    }

    fn lower_if_stmt(&mut self, stmt: &core::IfStmt) {
        if !stmt.condition_clauses.is_empty() {
            self.lower_if_condition_clauses(stmt, &stmt.condition_clauses);
            return;
        }
        if !stmt.pattern_clauses.is_empty() {
            self.lower_if_pattern_clauses(stmt, &stmt.pattern_clauses);
            return;
        }
        if let (Some(pattern), Some(value)) = (&stmt.pattern, &stmt.pattern_value) {
            self.lower_if_pattern_stmt(stmt, pattern, value);
            return;
        }
        if let Some(value) = &stmt.binding_value {
            self.lower_if_unwrap_stmt(stmt, value);
            return;
        }
        let Some(condition) = stmt.condition.as_ref() else {
            self.invariant(
                "if statement should have a condition or binding before lowering",
                stmt.span,
            );
            return;
        };
        let then_narrowing = self.type_narrowing_for_condition(condition, true);
        let else_narrowing = self.type_narrowing_for_condition(condition, false);
        let then_block = self.add_block();
        let else_block = self.add_block();
        let join_block = self.add_block();
        let cond = self.lower_expr(condition);
        self.terminate(ir::Terminator {
            span: Some(stmt.span),
            kind: ir::TerminatorKind::Branch {
                condition: cond,
                then_block,
                else_block,
            },
        });

        self.current_block = Some(then_block);
        self.push_scope();
        self.apply_type_narrowing(then_narrowing.as_ref());
        self.lower_block_statements(&stmt.then_block);
        self.pop_scope();
        let then_exits = self.current_block.is_none();
        if self.current_block.is_some() {
            self.terminate(ir::Terminator::goto(join_block));
        }

        self.current_block = Some(else_block);
        self.push_scope();
        self.apply_type_narrowing(else_narrowing.as_ref());
        if let Some(branch) = &stmt.else_branch {
            self.lower_else_branch(branch);
        }
        self.pop_scope();
        let else_exits = self.current_block.is_none();
        if self.current_block.is_some() {
            self.terminate(ir::Terminator::goto(join_block));
        }

        let join_used = self.block_has_predecessor(join_block);
        self.current_block = if join_used { Some(join_block) } else { None };
        if self.current_block.is_some() {
            if then_exits && !else_exits {
                self.apply_type_narrowing(else_narrowing.as_ref());
            } else if else_exits && !then_exits {
                self.apply_type_narrowing(then_narrowing.as_ref());
            }
        }
    }

    fn lower_if_condition_clauses(
        &mut self,
        stmt: &core::IfStmt,
        clauses: &[core::IfConditionClause],
    ) {
        let then_block = self.add_block();
        let else_block = self.add_block();
        let join_block = self.add_block();

        self.push_scope();
        self.lower_if_condition_clause_chain(clauses, then_block, else_block);

        self.current_block = Some(then_block);
        self.lower_block_statements(&stmt.then_block);
        self.pop_scope();
        if self.current_block.is_some() {
            self.terminate(ir::Terminator::goto(join_block));
        }

        self.current_block = Some(else_block);
        if let Some(branch) = &stmt.else_branch {
            self.lower_else_branch(branch);
        }
        if self.current_block.is_some() {
            self.terminate(ir::Terminator::goto(join_block));
        }

        let join_used = self.block_has_predecessor(join_block);
        self.current_block = if join_used { Some(join_block) } else { None };
    }

    fn lower_if_pattern_clauses(&mut self, stmt: &core::IfStmt, clauses: &[core::RefutableClause]) {
        let then_block = self.add_block();
        let else_block = self.add_block();
        let join_block = self.add_block();

        self.push_scope();
        self.lower_refutable_clause_chain(clauses, then_block, else_block);

        self.current_block = Some(then_block);
        self.lower_block_statements(&stmt.then_block);
        self.pop_scope();
        if self.current_block.is_some() {
            self.terminate(ir::Terminator::goto(join_block));
        }

        self.current_block = Some(else_block);
        if let Some(branch) = &stmt.else_branch {
            self.lower_else_branch(branch);
        }
        if self.current_block.is_some() {
            self.terminate(ir::Terminator::goto(join_block));
        }

        let join_used = self.block_has_predecessor(join_block);
        self.current_block = if join_used { Some(join_block) } else { None };
    }

    fn lower_if_pattern_stmt(&mut self, stmt: &core::IfStmt, pattern: &Pattern, value: &Expr) {
        let scrutinee = self.lower_expr(value);
        let plan = self.lower_pattern_plan(scrutinee, pattern);
        let then_block = self.add_block();
        let else_block = self.add_block();
        let join_block = self.add_block();

        self.terminate(ir::Terminator {
            span: Some(stmt.span),
            kind: ir::TerminatorKind::Branch {
                condition: plan.condition,
                then_block,
                else_block,
            },
        });

        self.current_block = Some(then_block);
        self.push_scope();
        self.apply_pending_bindings(plan.bindings);
        self.lower_block_statements(&stmt.then_block);
        self.pop_scope();
        if self.current_block.is_some() {
            self.terminate(ir::Terminator::goto(join_block));
        }

        self.current_block = Some(else_block);
        if let Some(branch) = &stmt.else_branch {
            self.lower_else_branch(branch);
        }
        if self.current_block.is_some() {
            self.terminate(ir::Terminator::goto(join_block));
        }

        let join_used = self.block_has_predecessor(join_block);
        self.current_block = if join_used { Some(join_block) } else { None };
    }

    fn lower_pattern_binding_stmt(&mut self, stmt: &core::PatternBindingStmt) {
        if !stmt.clauses.is_empty() {
            for clause in &stmt.clauses {
                let scrutinee = self.lower_expr(&clause.value);
                let plan = self.lower_pattern_plan(scrutinee, &clause.pattern);
                self.apply_pending_bindings(plan.bindings);
            }
            return;
        }

        let scrutinee = self.lower_expr(&stmt.value);
        let plan = self.lower_pattern_plan(scrutinee, &stmt.pattern);
        self.apply_pending_bindings(plan.bindings);
    }

    fn lower_let_else_stmt(&mut self, stmt: &core::LetElseStmt) {
        if !stmt.clauses.is_empty() {
            self.lower_let_else_clauses(stmt);
            return;
        }
        let scrutinee = self.lower_expr(&stmt.value);
        let plan = self.lower_pattern_plan(scrutinee, &stmt.pattern);
        let success_block = self.add_block();
        let failure_block = self.add_block();
        let continue_block = self.add_block();

        self.terminate(ir::Terminator {
            span: Some(stmt.span),
            kind: ir::TerminatorKind::Branch {
                condition: plan.condition,
                then_block: success_block,
                else_block: failure_block,
            },
        });

        self.current_block = Some(success_block);
        let bindings = self.apply_pending_bindings(plan.bindings);
        if self.current_block.is_some() {
            self.terminate(ir::Terminator::goto(continue_block));
        }

        self.current_block = Some(failure_block);
        self.lower_let_else_fallback(&stmt.else_block, &bindings, continue_block);

        self.current_block = Some(continue_block);
    }

    fn lower_let_else_clauses(&mut self, stmt: &core::LetElseStmt) {
        let failure_block = self.add_block();
        let success_block = self.add_block();
        let continue_block = self.add_block();

        let bindings =
            self.lower_refutable_clause_chain(&stmt.clauses, success_block, failure_block);
        if self.current_block.is_some() {
            self.terminate(ir::Terminator::goto(continue_block));
        }

        self.current_block = Some(failure_block);
        self.lower_let_else_fallback(&stmt.else_block, &bindings, continue_block);

        self.current_block = Some(continue_block);
    }

    fn lower_let_else_fallback(
        &mut self,
        block: &Block,
        bindings: &[AppliedBinding],
        continue_block: ir::BlockId,
    ) {
        let hidden = bindings
            .iter()
            .filter_map(|binding| {
                self.current_scope()
                    .remove(&binding.name)
                    .map(|local| (binding.name.clone(), local))
            })
            .collect::<Vec<_>>();

        let expected = if bindings.len() == 1 {
            Some(bindings[0].ty.clone())
        } else {
            let tail = block
                .statements
                .last()
                .and_then(|statement| match statement {
                    Stmt::Expr(statement) => Some(&statement.expr),
                    _ => None,
                });
            match tail {
                Some(Expr::TupleLiteral { .. }) => Some(ir::Type::Tuple(
                    bindings.iter().map(|binding| binding.ty.clone()).collect(),
                )),
                Some(Expr::RecordLiteral { .. })
                | Some(Expr::ContextualNew {
                    style: core::CallStyle::Brace,
                    ..
                }) => Some(ir::Type::Record(
                    bindings
                        .iter()
                        .map(|binding| ir::NamedType {
                            name: binding.name.clone(),
                            ty: binding.ty.clone(),
                        })
                        .collect(),
                )),
                _ => None,
            }
        };
        let value = self.lower_block_value_with_expected(block, expected.as_ref());

        for (name, local) in hidden {
            self.current_scope().insert(name, local);
        }

        if self.current_block.is_none() {
            return;
        }
        let Some(value) = value else {
            self.terminate(ir::Terminator::ret(Some(ir::Operand::Const(
                ir::Constant::Unit,
            ))));
            return;
        };
        if matches!(self.operand_type(&value), Some(ir::Type::Never)) {
            self.terminate(ir::Terminator::ret(Some(value)));
            return;
        }

        if let [binding] = bindings {
            self.push_statement(ir::Statement {
                span: Some(block.span),
                kind: ir::StatementKind::Assign {
                    target: ir::Place::Local(binding.local),
                    value: ir::RValue::Use(value),
                },
            });
        } else {
            let tuple = matches!(self.operand_type(&value), Some(ir::Type::Tuple(_)));
            for (index, binding) in bindings.iter().enumerate() {
                let field_name = if tuple {
                    format!("_{}", index + 1)
                } else {
                    binding.name.clone()
                };
                let field = self.emit_temp_from_rvalue(
                    ir::RValue::Field {
                        base: value.clone(),
                        name: field_name,
                    },
                    binding.ty.clone(),
                    Some(block.span),
                );
                self.push_statement(ir::Statement {
                    span: Some(block.span),
                    kind: ir::StatementKind::Assign {
                        target: ir::Place::Local(binding.local),
                        value: ir::RValue::Use(field),
                    },
                });
            }
        }
        self.terminate(ir::Terminator::goto(continue_block));
    }

    fn lower_refutable_clause_chain(
        &mut self,
        clauses: &[core::RefutableClause],
        success_target: ir::BlockId,
        failure_target: ir::BlockId,
    ) -> Vec<AppliedBinding> {
        let mut bindings = Vec::new();
        for (index, clause) in clauses.iter().enumerate() {
            let scrutinee = self.lower_expr(&clause.value);
            let plan = self.lower_pattern_plan(scrutinee, &clause.pattern);
            let success_block = if index + 1 == clauses.len() {
                success_target
            } else {
                self.add_block()
            };

            self.terminate(ir::Terminator {
                span: Some(clause.span),
                kind: ir::TerminatorKind::Branch {
                    condition: plan.condition,
                    then_block: success_block,
                    else_block: failure_target,
                },
            });

            self.current_block = Some(success_block);
            bindings.extend(self.apply_pending_bindings(plan.bindings));
            if index + 1 == clauses.len() {
                break;
            }
        }
        bindings
    }

    fn emit_panic(&mut self, message: &str, span: Span) {
        self.push_statement(ir::Statement {
            span: Some(span),
            kind: ir::StatementKind::Eval {
                value: ir::RValue::Call {
                    callee: ir::Callee::Intrinsic(ir::Intrinsic::Panic),
                    args: vec![ir::Operand::Const(ir::Constant::String(
                        message.to_string(),
                    ))],
                    structural: false,
                },
            },
        });
    }

    fn lower_if_condition_clause_chain(
        &mut self,
        clauses: &[core::IfConditionClause],
        success_target: ir::BlockId,
        failure_target: ir::BlockId,
    ) {
        for (index, clause) in clauses.iter().enumerate() {
            let success_block = if index + 1 == clauses.len() {
                success_target
            } else {
                self.add_block()
            };

            match clause {
                core::IfConditionClause::Let(clause) => {
                    let scrutinee = self.lower_expr(&clause.value);
                    let plan = self.lower_pattern_plan(scrutinee, &clause.pattern);
                    self.terminate(ir::Terminator {
                        span: Some(clause.span),
                        kind: ir::TerminatorKind::Branch {
                            condition: plan.condition,
                            then_block: success_block,
                            else_block: failure_target,
                        },
                    });
                    self.current_block = Some(success_block);
                    self.apply_pending_bindings(plan.bindings);
                }
                core::IfConditionClause::Expr(condition) => {
                    let narrowing = self.type_narrowing_for_condition(condition, true);
                    let cond = self.lower_expr(condition);
                    self.terminate(ir::Terminator {
                        span: Some(condition.span()),
                        kind: ir::TerminatorKind::Branch {
                            condition: cond,
                            then_block: success_block,
                            else_block: failure_target,
                        },
                    });
                    self.current_block = Some(success_block);
                    self.apply_type_narrowing(narrowing.as_ref());
                }
            }

            if index + 1 == clauses.len() {
                break;
            }
        }
    }

    fn lower_if_unwrap_stmt(&mut self, stmt: &core::IfStmt, value: &Expr) {
        let source = self.lower_expr(value);
        let source_local = self.add_temp(ir::Type::Unknown);
        self.push_statement(ir::Statement {
            span: Some(value.span()),
            kind: ir::StatementKind::Assign {
                target: ir::Place::Local(source_local),
                value: ir::RValue::Use(source),
            },
        });

        let then_block = self.add_block();
        let else_block = self.add_block();
        let join_block = self.add_block();

        let present = self.emit_temp_from_rvalue(
            ir::RValue::Call {
                callee: ir::Callee::Method {
                    receiver: ir::Operand::Copy(Box::new(ir::Place::Local(source_local))),
                    method: "isSuccess".to_string(),
                },
                args: Vec::new(),
                structural: false,
            },
            ir::Type::Bool,
            Some(stmt.span),
        );
        self.terminate(ir::Terminator {
            span: Some(stmt.span),
            kind: ir::TerminatorKind::Branch {
                condition: present,
                then_block,
                else_block,
            },
        });

        self.current_block = Some(then_block);
        let inner = self.emit_temp_from_rvalue(
            ir::RValue::Call {
                callee: ir::Callee::Intrinsic(ir::Intrinsic::ExtractSuccessValue),
                args: vec![ir::Operand::Copy(Box::new(ir::Place::Local(source_local)))],
                structural: false,
            },
            ir::Type::Unknown,
            Some(stmt.span),
        );
        self.push_scope();
        self.bind_unwrap_values(&stmt.bindings, inner);
        self.lower_block_statements(&stmt.then_block);
        self.pop_scope();
        if self.current_block.is_some() {
            self.terminate(ir::Terminator::goto(join_block));
        }

        self.current_block = Some(else_block);
        if let Some(branch) = &stmt.else_branch {
            self.lower_else_branch(branch);
        }
        if self.current_block.is_some() {
            self.terminate(ir::Terminator::goto(join_block));
        }

        let join_used = self.block_has_predecessor(join_block);
        self.current_block = if join_used { Some(join_block) } else { None };
    }

    fn lower_else_branch(&mut self, branch: &ElseBranch) {
        match branch {
            ElseBranch::If(stmt) => self.lower_if_stmt(stmt),
            ElseBranch::Block(block) => {
                self.lower_block_statements(block);
            }
        }
    }

    fn lower_if_stmt_tail_value(
        &mut self,
        stmt: &core::IfStmt,
        expected: Option<&ir::Type>,
    ) -> Option<ir::Operand> {
        let else_branch = stmt.else_branch.as_ref()?;
        let result = self.add_temp(expected.cloned().unwrap_or(ir::Type::Unknown));
        let then_block = self.add_block();
        let else_block = self.add_block();
        let join_block = self.add_block();
        let mut then_scope_pushed = false;
        let then_narrowing = stmt
            .condition
            .as_ref()
            .and_then(|condition| self.type_narrowing_for_condition(condition, true));
        let else_narrowing = stmt
            .condition
            .as_ref()
            .and_then(|condition| self.type_narrowing_for_condition(condition, false));

        if !stmt.condition_clauses.is_empty() {
            self.push_scope();
            then_scope_pushed = true;
            self.lower_if_condition_clause_chain(&stmt.condition_clauses, then_block, else_block);
        } else if !stmt.pattern_clauses.is_empty() {
            self.push_scope();
            then_scope_pushed = true;
            self.lower_refutable_clause_chain(&stmt.pattern_clauses, then_block, else_block);
        } else if let (Some(pattern), Some(value)) = (&stmt.pattern, &stmt.pattern_value) {
            let scrutinee = self.lower_expr(value);
            let plan = self.lower_pattern_plan(scrutinee, pattern);
            self.terminate(ir::Terminator {
                span: Some(stmt.span),
                kind: ir::TerminatorKind::Branch {
                    condition: plan.condition,
                    then_block,
                    else_block,
                },
            });
            self.current_block = Some(then_block);
            self.push_scope();
            then_scope_pushed = true;
            self.apply_pending_bindings(plan.bindings);
        } else if let Some(value) = &stmt.binding_value {
            let source = self.lower_expr(value);
            let source_local = self.add_temp(ir::Type::Unknown);
            self.push_statement(ir::Statement {
                span: Some(value.span()),
                kind: ir::StatementKind::Assign {
                    target: ir::Place::Local(source_local),
                    value: ir::RValue::Use(source),
                },
            });
            let present = self.emit_temp_from_rvalue(
                ir::RValue::Call {
                    callee: ir::Callee::Method {
                        receiver: ir::Operand::Copy(Box::new(ir::Place::Local(source_local))),
                        method: "isSuccess".to_string(),
                    },
                    args: Vec::new(),
                    structural: false,
                },
                ir::Type::Bool,
                Some(stmt.span),
            );
            self.terminate(ir::Terminator {
                span: Some(stmt.span),
                kind: ir::TerminatorKind::Branch {
                    condition: present,
                    then_block,
                    else_block,
                },
            });
            self.current_block = Some(then_block);
            let inner = self.emit_temp_from_rvalue(
                ir::RValue::Call {
                    callee: ir::Callee::Intrinsic(ir::Intrinsic::ExtractSuccessValue),
                    args: vec![ir::Operand::Copy(Box::new(ir::Place::Local(source_local)))],
                    structural: false,
                },
                ir::Type::Unknown,
                Some(stmt.span),
            );
            self.push_scope();
            then_scope_pushed = true;
            self.bind_unwrap_values(&stmt.bindings, inner);
        } else if let Some(condition) = &stmt.condition {
            let cond = self.lower_expr(condition);
            self.terminate(ir::Terminator {
                span: Some(stmt.span),
                kind: ir::TerminatorKind::Branch {
                    condition: cond,
                    then_block,
                    else_block,
                },
            });
            self.current_block = Some(then_block);
            if then_narrowing.is_some() {
                self.push_scope();
                then_scope_pushed = true;
                self.apply_type_narrowing(then_narrowing.as_ref());
            }
        } else {
            return None;
        }

        if let Some(value) = self.lower_block_value_with_expected(&stmt.then_block, expected) {
            self.assign_if_result(result, value, stmt.then_block.span);
        }
        if then_scope_pushed {
            self.pop_scope();
        }
        if self.current_block.is_some() {
            self.terminate(ir::Terminator::goto(join_block));
        }

        self.current_block = Some(else_block);
        let else_scope_pushed = else_narrowing.is_some();
        if else_scope_pushed {
            self.push_scope();
            self.apply_type_narrowing(else_narrowing.as_ref());
        }
        let else_value = match else_branch {
            ElseBranch::If(stmt) => self.lower_if_stmt_tail_value(stmt, expected),
            ElseBranch::Block(block) => self.lower_block_value_with_expected(block, expected),
        };
        if else_scope_pushed {
            self.pop_scope();
        }
        if let Some(value) = else_value {
            self.assign_if_result(result, value, stmt.span);
        }
        if self.current_block.is_some() {
            self.terminate(ir::Terminator::goto(join_block));
        }

        let join_used = self.block_has_predecessor(join_block);
        self.current_block = if join_used { Some(join_block) } else { None };
        Some(ir::Operand::Copy(Box::new(ir::Place::Local(result))))
    }

    fn assign_if_result(&mut self, target: ir::LocalId, value: ir::Operand, span: Span) {
        self.push_statement(ir::Statement {
            span: Some(span),
            kind: ir::StatementKind::Assign {
                target: ir::Place::Local(target),
                value: ir::RValue::Use(value),
            },
        });
    }

    fn lower_while_stmt(&mut self, stmt: &core::WhileStmt) {
        let cond_block = self.add_block();
        let body_block = self.add_block();
        let exit_block = self.add_block();

        self.terminate(ir::Terminator::goto(cond_block));

        self.push_scope();
        self.current_block = Some(cond_block);
        self.lower_if_condition_clause_chain(&stmt.condition_clauses, body_block, exit_block);

        self.loop_exits.push(exit_block);
        self.loop_continues.push(cond_block);
        self.current_block = Some(body_block);
        self.lower_block_statements(&stmt.body);
        if self.current_block.is_some() {
            self.terminate(ir::Terminator::goto(cond_block));
        }
        self.loop_exits.pop();
        self.loop_continues.pop();
        self.pop_scope();

        self.current_block = Some(exit_block);
    }

    fn lower_for_stmt(&mut self, stmt: &core::ForStmt) {
        if stmt.bindings.is_empty() {
            let body_block = self.add_block();
            let exit_block = self.add_block();
            self.terminate(ir::Terminator::goto(body_block));
            self.loop_exits.push(exit_block);
            self.loop_continues.push(body_block);
            self.current_block = Some(body_block);
            self.lower_block_statements(&stmt.body);
            if self.current_block.is_some() {
                self.terminate(ir::Terminator::goto(body_block));
            }
            self.loop_exits.pop();
            self.loop_continues.pop();
            self.current_block = Some(exit_block);
            return;
        }
        self.lower_for_bindings(
            &stmt.bindings,
            &|this| this.lower_block_statements(&stmt.body),
            stmt.span,
        );
    }

    fn lower_for_yield_expr(
        &mut self,
        bindings: &[core::ForBinding],
        yield_body: &Block,
        span: Span,
    ) -> ir::Operand {
        if let Some(family) = self.first_lifted_for_yield_family(bindings) {
            return self.lower_lifted_for_yield_bindings(bindings, yield_body, &family, span);
        }

        let result = self.add_temp(ir::Type::list(ir::Type::Unknown));
        self.push_statement(ir::Statement {
            span: Some(span),
            kind: ir::StatementKind::Assign {
                target: ir::Place::Local(result),
                value: ir::RValue::List(Vec::new()),
            },
        });

        if bindings.is_empty() {
            let body_block = self.add_block();
            let exit_block = self.add_block();
            self.terminate(ir::Terminator::goto(body_block));
            self.loop_exits.push(exit_block);
            self.loop_continues.push(body_block);
            self.current_block = Some(body_block);
            let yielded = self
                .lower_block_value(yield_body)
                .unwrap_or(ir::Operand::Const(ir::Constant::Unit));
            self.push_statement(ir::Statement {
                span: Some(span),
                kind: ir::StatementKind::Assign {
                    target: ir::Place::Local(result),
                    value: ir::RValue::Call {
                        callee: ir::Callee::Intrinsic(ir::Intrinsic::ListAppend),
                        args: vec![
                            ir::Operand::Copy(Box::new(ir::Place::Local(result))),
                            yielded,
                        ],
                        structural: false,
                    },
                },
            });
            if self.current_block.is_some() {
                self.terminate(ir::Terminator::goto(body_block));
            }
            self.loop_exits.pop();
            self.loop_continues.pop();
            self.current_block = Some(exit_block);
            return ir::Operand::Copy(Box::new(ir::Place::Local(result)));
        }

        self.lower_for_bindings(
            bindings,
            &|this| {
                let yielded = this
                    .lower_block_value(yield_body)
                    .unwrap_or(ir::Operand::Const(ir::Constant::Unit));
                this.push_statement(ir::Statement {
                    span: Some(span),
                    kind: ir::StatementKind::Assign {
                        target: ir::Place::Local(result),
                        value: ir::RValue::Call {
                            callee: ir::Callee::Intrinsic(ir::Intrinsic::ListAppend),
                            args: vec![
                                ir::Operand::Copy(Box::new(ir::Place::Local(result))),
                                yielded,
                            ],
                            structural: false,
                        },
                    },
                });
            },
            span,
        );

        ir::Operand::Copy(Box::new(ir::Place::Local(result)))
    }

    fn first_lifted_for_yield_family(
        &self,
        bindings: &[core::ForBinding],
    ) -> Option<LiftedIrFamily> {
        bindings.iter().find_map(|binding| {
            let iterable = binding.iterable.as_ref()?;
            let source_ty = self.infer_expr_type(iterable);
            known_lifted_ir_type(&source_ty).map(|(family, _)| family)
        })
    }

    fn lower_lifted_for_yield_bindings(
        &mut self,
        bindings: &[core::ForBinding],
        yield_body: &Block,
        family: &LiftedIrFamily,
        span: Span,
    ) -> ir::Operand {
        let Some(generator_index) = bindings
            .iter()
            .position(|binding| binding.iterable.is_some())
        else {
            return self
                .lower_block_value(yield_body)
                .unwrap_or(ir::Operand::Const(ir::Constant::Unit));
        };

        if generator_index > 0 {
            let result = self.add_temp(wrap_lifted_ir_type(family, ir::Type::Unknown));
            self.lower_for_bindings(
                &bindings[..generator_index],
                &|this| {
                    let value = this.lower_lifted_for_yield_bindings(
                        &bindings[generator_index..],
                        yield_body,
                        family,
                        span,
                    );
                    this.push_statement(ir::Statement {
                        span: Some(span),
                        kind: ir::StatementKind::Assign {
                            target: ir::Place::Local(result),
                            value: ir::RValue::Use(value),
                        },
                    });
                },
                span,
            );
            return ir::Operand::Copy(Box::new(ir::Place::Local(result)));
        }

        self.lower_lifted_for_yield_generator(
            &bindings[0],
            &bindings[1..],
            yield_body,
            family,
            span,
        )
    }

    fn lower_lifted_for_yield_generator(
        &mut self,
        binding: &core::ForBinding,
        rest: &[core::ForBinding],
        yield_body: &Block,
        family: &LiftedIrFamily,
        span: Span,
    ) -> ir::Operand {
        let Some(source_expr) = binding.iterable.as_ref() else {
            self.invariant(
                "lifted for-yield generator should have an iterable source",
                binding.span,
            );
            return ir::Operand::Const(ir::Constant::Unit);
        };
        let source_ty = self.infer_expr_type(source_expr);
        let Some((_, inner_ty)) = known_lifted_ir_type(&source_ty) else {
            self.invariant(
                "lifted for-yield source should have a lifted type before lowering",
                source_expr.span(),
            );
            return self.lower_expr(source_expr);
        };

        let source = self.lower_expr_with_expected(source_expr, Some(&source_ty));
        let rest_has_generator = rest.iter().any(|binding| binding.iterable.is_some());
        let method = if rest_has_generator { "flatMap" } else { "map" };
        let closure = self.lower_lifted_for_yield_closure(
            binding,
            rest,
            yield_body,
            family.clone(),
            inner_ty,
            span,
        );
        let closure_operand = self.emit_temp_from_rvalue(closure, ir::Type::Unknown, Some(span));
        self.emit_temp_from_rvalue(
            ir::RValue::Call {
                callee: ir::Callee::Method {
                    receiver: source,
                    method: method.to_string(),
                },
                args: vec![closure_operand],
                structural: false,
            },
            wrap_lifted_ir_type(family, ir::Type::Unknown),
            Some(span),
        )
    }

    fn lower_lifted_for_yield_closure(
        &mut self,
        binding: &core::ForBinding,
        rest: &[core::ForBinding],
        yield_body: &Block,
        family: LiftedIrFamily,
        param_ty: ir::Type,
        span: Span,
    ) -> ir::RValue {
        let nested_name = format!(
            "foryield${}${}",
            self.function_id.0,
            self.function().blocks.len()
        );
        let mut nested =
            ir::Function::new(nested_name, ir::FunctionKind::Lambda, ir::Type::Unknown);
        nested.span = Some(span);
        nested.add_param("__for_item".to_string(), param_ty);
        let function_id = self.program.add_function(nested);
        let capture_sources = self.visible_capture_sources(None);
        let captures = {
            let mut lowerer = FunctionLowerer::new(
                self.program,
                self.core_bodies,
                function_id,
                self.globals,
                self.functions,
                self.case_fields,
                self.type_aliases,
                self.diagnostics,
            )
            .with_capture_sources(capture_sources);
            if let Some(local_id) = lowerer.function().params.first().copied() {
                let item = ir::Operand::Copy(Box::new(ir::Place::Local(local_id)));
                lowerer.bind_for_values(binding, item);
            }
            let value = lowerer.lower_lifted_for_yield_bindings(rest, yield_body, &family, span);
            if lowerer.current_block.is_some() {
                lowerer.terminate(ir::Terminator::ret(Some(value)));
            }
            lowerer.finish_closure_captures()
        };
        ir::RValue::Closure {
            function: function_id,
            captures,
        }
    }

    fn lower_for_bindings(
        &mut self,
        bindings: &[core::ForBinding],
        body: &dyn Fn(&mut Self),
        span: Span,
    ) {
        if bindings.is_empty() {
            body(self);
            return;
        }

        let first = &bindings[0];
        if !first.values.is_empty() && first.iterable.is_none() {
            if let Some(pattern) = &first.pattern {
                let scrutinee = self.lower_expr(&first.values[0]);
                let plan = self.lower_pattern_plan(scrutinee, pattern);
                let success_block = self.add_block();
                let failure_block = self.add_block();

                self.terminate(ir::Terminator {
                    span: Some(first.span),
                    kind: ir::TerminatorKind::Branch {
                        condition: plan.condition,
                        then_block: success_block,
                        else_block: failure_block,
                    },
                });

                self.current_block = Some(success_block);
                self.apply_pending_bindings(plan.bindings);
                self.lower_for_bindings(&bindings[1..], body, span);
                let success_current = self.current_block;

                self.current_block = Some(failure_block);
                self.emit_panic("for pattern did not match", first.span);
                self.terminate(ir::Terminator {
                    span: Some(first.span),
                    kind: ir::TerminatorKind::Unreachable,
                });
                self.current_block = success_current;
                return;
            }
            if first.destructure.is_some() && first.values.len() == 1 {
                let source_value = self.lower_expr(&first.values[0]);
                let field_names = matches!(first.destructure, Some(DestructureKind::Record))
                    .then(|| self.destructure_field_names_from_bindings(&first.bindings));
                for (index, binding) in first.bindings.iter().enumerate() {
                    if binding.name == "_" {
                        continue;
                    }
                    let field_value = match first.destructure {
                        Some(DestructureKind::Record) => self.emit_temp_from_rvalue(
                            ir::RValue::Field {
                                base: source_value.clone(),
                                name: field_names
                                    .as_ref()
                                    .and_then(|fields| fields.get(index).cloned())
                                    .unwrap_or_else(|| format!("_{}", index + 1)),
                            },
                            ir::Type::Unknown,
                            Some(binding.span),
                        ),
                        Some(DestructureKind::Tuple) => self.emit_temp_from_rvalue(
                            ir::RValue::Field {
                                base: source_value.clone(),
                                name: format!("_{}", index + 1),
                            },
                            ir::Type::Unknown,
                            Some(binding.span),
                        ),
                        None => source_value.clone(),
                    };
                    self.bind_loop_binding(binding, field_value);
                }
            } else {
                for (index, binding) in first.bindings.iter().enumerate() {
                    if binding.name == "_" {
                        continue;
                    }
                    let ty = binding
                        .ty
                        .as_ref()
                        .map(|ty| self.lower_type_ref(ty))
                        .unwrap_or(ir::Type::Unknown);
                    let local_id = self.add_local(
                        binding.name.clone(),
                        ty,
                        binding.mutable,
                        ir::LocalKind::Binding,
                    );
                    self.current_scope().insert(binding.name.clone(), local_id);
                    if let Some(value) = first.values.get(index) {
                        let operand = self.lower_expr(value);
                        self.push_statement(ir::Statement {
                            span: Some(binding.span),
                            kind: ir::StatementKind::Assign {
                                target: ir::Place::Local(local_id),
                                value: ir::RValue::Use(operand),
                            },
                        });
                    }
                }
            }
            self.lower_for_bindings(&bindings[1..], body, span);
            return;
        }

        let Some(iterable) = &first.iterable else {
            self.invariant(
                "for binding should have an iterable source before lowering",
                first.span,
            );
            return;
        };

        let iter_value = self.lower_expr(iterable);
        let iter_local = self.add_temp(ir::Type::Unknown);
        self.push_statement(ir::Statement {
            span: Some(iterable.span()),
            kind: ir::StatementKind::Assign {
                target: ir::Place::Local(iter_local),
                value: ir::RValue::Call {
                    callee: ir::Callee::Intrinsic(ir::Intrinsic::IterInit),
                    args: vec![iter_value],
                    structural: false,
                },
            },
        });

        let cond_block = self.add_block();
        let body_block = self.add_block();
        let exit_block = self.add_block();
        self.terminate(ir::Terminator::goto(cond_block));

        self.current_block = Some(cond_block);
        let has_next = self.emit_temp_from_rvalue(
            ir::RValue::Call {
                callee: ir::Callee::Intrinsic(ir::Intrinsic::IterHasNext),
                args: vec![ir::Operand::Copy(Box::new(ir::Place::Local(iter_local)))],
                structural: false,
            },
            ir::Type::Bool,
            Some(first.span),
        );
        self.terminate(ir::Terminator {
            span: Some(span),
            kind: ir::TerminatorKind::Branch {
                condition: has_next,
                then_block: body_block,
                else_block: exit_block,
            },
        });

        self.loop_exits.push(exit_block);
        self.loop_continues.push(cond_block);
        self.current_block = Some(body_block);
        self.push_scope();
        let item = self.emit_temp_from_rvalue(
            ir::RValue::Call {
                callee: ir::Callee::Intrinsic(ir::Intrinsic::IterNext),
                args: vec![ir::Operand::Copy(Box::new(ir::Place::Local(iter_local)))],
                structural: false,
            },
            ir::Type::Unknown,
            Some(first.span),
        );
        if let Some(pattern) = &first.pattern {
            let plan = self.lower_pattern_plan(item, pattern);
            let success_block = self.add_block();
            let failure_block = self.add_block();

            self.terminate(ir::Terminator {
                span: Some(first.span),
                kind: ir::TerminatorKind::Branch {
                    condition: plan.condition,
                    then_block: success_block,
                    else_block: failure_block,
                },
            });

            self.current_block = Some(success_block);
            self.apply_pending_bindings(plan.bindings);
            self.lower_for_bindings(&bindings[1..], body, span);
            self.pop_scope();
            if self.current_block.is_some() {
                self.terminate(ir::Terminator::goto(cond_block));
            }

            self.current_block = Some(failure_block);
            self.emit_panic("for pattern did not match", first.span);
            self.terminate(ir::Terminator {
                span: Some(first.span),
                kind: ir::TerminatorKind::Unreachable,
            });
        } else {
            self.bind_for_values(first, item);
            self.lower_for_bindings(&bindings[1..], body, span);
            self.pop_scope();
            if self.current_block.is_some() {
                self.terminate(ir::Terminator::goto(cond_block));
            }
        }
        self.loop_exits.pop();
        self.loop_continues.pop();
        self.current_block = Some(exit_block);
    }

    fn bind_for_values(&mut self, binding: &core::ForBinding, item: ir::Operand) {
        if binding.pattern.is_some() {
            self.invariant(
                "pattern-based for bindings should branch before local binding",
                binding.span,
            );
            return;
        }
        match binding.destructure {
            None => {
                if let Some(local) = binding.bindings.first() {
                    self.bind_loop_binding(local, item);
                }
            }
            Some(DestructureKind::Tuple) => {
                for (index, local) in binding.bindings.iter().enumerate() {
                    let field_value = self.emit_temp_from_rvalue(
                        ir::RValue::Field {
                            base: item.clone(),
                            name: format!("_{}", index + 1),
                        },
                        ir::Type::Unknown,
                        Some(local.span),
                    );
                    self.bind_loop_binding(local, field_value);
                }
            }
            Some(DestructureKind::Record) => {
                let field_names = self.destructure_field_names_from_bindings(&binding.bindings);
                for (index, local) in binding.bindings.iter().enumerate() {
                    let field_value = self.emit_temp_from_rvalue(
                        ir::RValue::Field {
                            base: item.clone(),
                            name: field_names
                                .get(index)
                                .cloned()
                                .unwrap_or_else(|| format!("_{}", index + 1)),
                        },
                        ir::Type::Unknown,
                        Some(local.span),
                    );
                    self.bind_loop_binding(local, field_value);
                }
            }
        }
    }

    fn bind_lambda_destructure_param(
        &mut self,
        destructure: &ast::LambdaParamDestructure,
        item: ir::Operand,
    ) {
        let field_names = (destructure.kind == DestructureKind::Record)
            .then(|| self.destructure_field_names_from_bindings(&destructure.bindings));
        for (index, binding) in destructure.bindings.iter().enumerate() {
            if binding.name == "_" {
                continue;
            }
            let field_value = self.emit_temp_from_rvalue(
                ir::RValue::Field {
                    base: item.clone(),
                    name: field_names
                        .as_ref()
                        .and_then(|fields| fields.get(index).cloned())
                        .unwrap_or_else(|| format!("_{}", index + 1)),
                },
                ir::Type::Unknown,
                Some(binding.span),
            );
            let ty = binding
                .ty
                .as_ref()
                .map(|ty| self.lower_type_ref(ty))
                .unwrap_or(ir::Type::Unknown);
            let local_id = self.add_local(binding.name.clone(), ty, false, ir::LocalKind::Binding);
            self.current_scope().insert(binding.name.clone(), local_id);
            self.push_statement(ir::Statement {
                span: Some(binding.span),
                kind: ir::StatementKind::Assign {
                    target: ir::Place::Local(local_id),
                    value: ir::RValue::Use(field_value),
                },
            });
        }
    }

    fn bind_loop_binding(&mut self, binding: &ast::Binding, value: ir::Operand) {
        if binding.name == "_" {
            return;
        }
        let ty = binding
            .ty
            .as_ref()
            .map(|ty| self.lower_type_ref(ty))
            .unwrap_or(ir::Type::Unknown);
        let local_id = self.add_local(
            binding.name.clone(),
            ty,
            binding.mutable,
            ir::LocalKind::Binding,
        );
        self.current_scope().insert(binding.name.clone(), local_id);
        self.push_statement(ir::Statement {
            span: Some(binding.span),
            kind: ir::StatementKind::Assign {
                target: ir::Place::Local(local_id),
                value: ir::RValue::Use(value),
            },
        });
    }

    fn destructure_field_names(&self, _expr: &Expr, bindings: &[ast::Binding]) -> Vec<String> {
        bindings
            .iter()
            .map(|binding| {
                binding
                    .field_name
                    .clone()
                    .or_else(|| (binding.name != "_").then(|| binding.name.clone()))
                    .unwrap_or_else(|| "_".to_string())
            })
            .collect()
    }

    fn destructure_field_names_from_bindings(&self, bindings: &[ast::Binding]) -> Vec<String> {
        bindings
            .iter()
            .map(|binding| {
                binding
                    .field_name
                    .clone()
                    .or_else(|| (binding.name != "_").then(|| binding.name.clone()))
                    .unwrap_or_else(|| "_".to_string())
            })
            .collect()
    }

    fn bind_unwrap_values(&mut self, bindings: &[ast::Binding], item: ir::Operand) {
        if bindings.len() <= 1 {
            if let Some(binding) = bindings.first() {
                self.bind_unwrap_binding(binding, item);
            }
            return;
        }

        for (index, binding) in bindings.iter().enumerate() {
            let field_value = self.emit_temp_from_rvalue(
                ir::RValue::Field {
                    base: item.clone(),
                    name: format!("_{}", index + 1),
                },
                ir::Type::Unknown,
                Some(binding.span),
            );
            self.bind_unwrap_binding(binding, field_value);
        }
    }

    fn bind_unwrap_binding(&mut self, binding: &ast::Binding, value: ir::Operand) {
        if binding.name == "_" {
            return;
        }
        let ty = binding
            .ty
            .as_ref()
            .map(|ty| self.lower_type_ref(ty))
            .unwrap_or(ir::Type::Unknown);
        let local_id = self.add_local(binding.name.clone(), ty, false, ir::LocalKind::Binding);
        self.current_scope().insert(binding.name.clone(), local_id);
        self.push_statement(ir::Statement {
            span: Some(binding.span),
            kind: ir::StatementKind::Assign {
                target: ir::Place::Local(local_id),
                value: ir::RValue::Use(value),
            },
        });
    }

    fn lower_match_stmt(&mut self, stmt: &core::MatchStmt) {
        let scrutinee = self.lower_expr(&stmt.value);
        let join_block = self.add_block();
        self.current_block = self.current_block.or(Some(self.function().entry));
        let first_case_block = self.current_block.expect("match entry block");
        let mut case_blocks = vec![first_case_block];
        case_blocks.extend((1..stmt.cases.len()).map(|_| self.add_block()));
        case_blocks.push(join_block);

        if stmt.cases.is_empty() {
            self.terminate(ir::Terminator::goto(join_block));
        }

        for (index, case) in stmt.cases.iter().enumerate() {
            self.current_block = Some(case_blocks[index]);
            let body_block = self.add_block();
            let pattern_fail_block = case_blocks[index + 1];
            let guard_fail_index = (index + case.remaining_alternatives + 1).min(stmt.cases.len());
            let guard_fail_block = case_blocks[guard_fail_index];
            self.lower_match_case(
                scrutinee.clone(),
                case,
                body_block,
                pattern_fail_block,
                guard_fail_block,
                None,
                join_block,
                stmt.span,
                None,
            );
        }
        self.current_block = Some(join_block);
    }

    fn lower_match_expr(
        &mut self,
        value: &Expr,
        cases: &[core::MatchCase],
        span: Span,
    ) -> ir::Operand {
        self.lower_match_expr_with_expected(value, cases, span, None)
    }

    fn lower_match_expr_with_expected(
        &mut self,
        value: &Expr,
        cases: &[core::MatchCase],
        span: Span,
        expected: Option<&ir::Type>,
    ) -> ir::Operand {
        let scrutinee = self.lower_expr(value);
        let result = self.add_temp(expected.cloned().unwrap_or(ir::Type::Unknown));
        let join_block = self.add_block();
        self.current_block = self.current_block.or(Some(self.function().entry));
        let first_case_block = self.current_block.expect("match entry block");
        let mut case_blocks = vec![first_case_block];
        case_blocks.extend((1..cases.len()).map(|_| self.add_block()));
        let unmatched_block = self.add_block();
        case_blocks.push(unmatched_block);

        if cases.is_empty() {
            self.terminate(ir::Terminator::goto(unmatched_block));
        }

        for (index, case) in cases.iter().enumerate() {
            self.current_block = Some(case_blocks[index]);
            let body_block = self.add_block();
            let pattern_fail_block = case_blocks[index + 1];
            let guard_fail_index = (index + case.remaining_alternatives + 1).min(cases.len());
            let guard_fail_block = case_blocks[guard_fail_index];
            self.lower_match_case(
                scrutinee.clone(),
                case,
                body_block,
                pattern_fail_block,
                guard_fail_block,
                Some(result),
                join_block,
                span,
                expected,
            );
        }

        self.current_block = Some(unmatched_block);
        if let Some(block) = self.current_block_mut() {
            block.push(ir::Statement {
                span: Some(span),
                kind: ir::StatementKind::Assign {
                    target: ir::Place::Local(result),
                    value: ir::RValue::Use(ir::Operand::Const(ir::Constant::Unit)),
                },
            });
            block.set_terminator(ir::Terminator::goto(join_block));
        }

        self.current_block = Some(join_block);
        ir::Operand::Copy(Box::new(ir::Place::Local(result)))
    }

    fn lower_match_case(
        &mut self,
        scrutinee: ir::Operand,
        case: &core::MatchCase,
        body_block: ir::BlockId,
        pattern_fail_block: ir::BlockId,
        guard_fail_block: ir::BlockId,
        result_target: Option<ir::LocalId>,
        join_block: ir::BlockId,
        span: Span,
        expected: Option<&ir::Type>,
    ) {
        let plan = self.lower_pattern_plan(scrutinee, &case.pattern);
        let mut condition = plan.condition;
        if let Some(guard) = &case.guard {
            let guard_block = self.add_block();
            self.terminate(ir::Terminator {
                span: Some(case.span),
                kind: ir::TerminatorKind::Branch {
                    condition,
                    then_block: guard_block,
                    else_block: pattern_fail_block,
                },
            });
            self.current_block = Some(guard_block);
            self.push_scope();
            self.apply_pending_bindings(plan.bindings.clone());
            condition = self.lower_expr(guard);
            self.terminate(ir::Terminator {
                span: Some(case.span),
                kind: ir::TerminatorKind::Branch {
                    condition,
                    then_block: body_block,
                    else_block: guard_fail_block,
                },
            });
            self.pop_scope();
        } else {
            self.terminate(ir::Terminator {
                span: Some(case.span),
                kind: ir::TerminatorKind::Branch {
                    condition,
                    then_block: body_block,
                    else_block: pattern_fail_block,
                },
            });
        }

        self.current_block = Some(body_block);
        self.push_scope();
        self.apply_pending_bindings(plan.bindings);
        match &case.body {
            MatchCaseBody::Block(block) => {
                if let Some(target) = result_target {
                    let value = self
                        .lower_block_value_with_expected(block, expected)
                        .unwrap_or(ir::Operand::Const(ir::Constant::Unit));
                    self.assign_match_result(target, value, span);
                } else {
                    self.lower_block_statements(block);
                }
            }
            MatchCaseBody::Expr(expr) => {
                let value = self.lower_expr_with_expected(expr, expected);
                if let Some(target) = result_target {
                    self.assign_match_result(target, value, span);
                } else {
                    self.push_statement(ir::Statement {
                        span: Some(case.span),
                        kind: ir::StatementKind::Eval {
                            value: ir::RValue::Use(value),
                        },
                    });
                }
            }
        }
        self.pop_scope();
        if self.current_block.is_some() {
            self.terminate(ir::Terminator::goto(join_block));
        }
    }

    fn assign_match_result(&mut self, target: ir::LocalId, value: ir::Operand, span: Span) {
        self.push_statement(ir::Statement {
            span: Some(span),
            kind: ir::StatementKind::Assign {
                target: ir::Place::Local(target),
                value: ir::RValue::Use(value),
            },
        });
    }

    fn lower_pattern_plan(&mut self, scrutinee: ir::Operand, pattern: &Pattern) -> PatternPlan {
        match pattern {
            Pattern::Wildcard { .. } => PatternPlan::always_true(),
            Pattern::Extract { inner, span } => {
                let payload_ty = self
                    .operand_type(&scrutinee)
                    .and_then(|ty| unwrap_lifted_ir_type(&ty).map(|(_, inner)| inner))
                    .unwrap_or(ir::Type::Unknown);
                let base_condition = self.emit_temp_from_rvalue(
                    ir::RValue::Call {
                        callee: ir::Callee::Intrinsic(ir::Intrinsic::ExtractSuccessIsSet),
                        args: vec![scrutinee.clone()],
                        structural: false,
                    },
                    ir::Type::Bool,
                    Some(*span),
                );
                if let Pattern::Binding { name, .. } = inner.as_ref() {
                    let bindings = if name == "_" {
                        Vec::new()
                    } else {
                        vec![PendingBinding {
                            name: name.clone(),
                            ty: payload_ty,
                            source: PendingBindingSource::RValue(ir::RValue::Call {
                                callee: ir::Callee::Intrinsic(ir::Intrinsic::ExtractSuccessValue),
                                args: vec![scrutinee],
                                structural: false,
                            }),
                        }]
                    };
                    return PatternPlan {
                        condition: base_condition,
                        bindings,
                    };
                }
                if matches!(inner.as_ref(), Pattern::Wildcard { .. }) {
                    return PatternPlan {
                        condition: base_condition,
                        bindings: Vec::new(),
                    };
                }
                let payload = self.emit_temp_from_rvalue(
                    ir::RValue::Call {
                        callee: ir::Callee::Intrinsic(ir::Intrinsic::ExtractSuccessValue),
                        args: vec![scrutinee],
                        structural: false,
                    },
                    payload_ty,
                    Some(*span),
                );
                let inner_plan = self.lower_pattern_plan(payload, inner);
                PatternPlan {
                    condition: self
                        .combine_conditions(vec![base_condition, inner_plan.condition], *span),
                    bindings: inner_plan.bindings,
                }
            }
            Pattern::Alias { inner, name, .. } => {
                let mut plan = self.lower_pattern_plan(scrutinee.clone(), inner);
                if name != "_" {
                    let source_ty = self.operand_type(&scrutinee).unwrap_or(ir::Type::Unknown);
                    let target_ty = self.whole_pattern_binding_type(inner, &scrutinee);
                    let source = if target_ty != ir::Type::Unknown && target_ty != source_ty {
                        PendingBindingSource::RValue(ir::RValue::Cast {
                            operand: scrutinee,
                            ty: target_ty.clone(),
                        })
                    } else {
                        PendingBindingSource::Operand(scrutinee)
                    };
                    plan.bindings.push(PendingBinding {
                        name: name.clone(),
                        ty: target_ty,
                        source,
                    });
                }
                plan
            }
            Pattern::Binding { name, .. } => {
                if name == "_" {
                    PatternPlan::always_true()
                } else {
                    PatternPlan {
                        condition: self.bool_const(true),
                        bindings: vec![PendingBinding {
                            name: name.clone(),
                            ty: self.operand_type(&scrutinee).unwrap_or(ir::Type::Unknown),
                            source: PendingBindingSource::Operand(scrutinee),
                        }],
                    }
                }
            }
            Pattern::Literal { value, span } => {
                let right = self.lower_pattern_literal_expr(value);
                let condition = self.emit_temp_from_rvalue(
                    ir::RValue::Binary {
                        op: ir::BinaryOp::Eq,
                        left: scrutinee,
                        right,
                    },
                    ir::Type::Bool,
                    Some(*span),
                );
                PatternPlan {
                    condition,
                    bindings: Vec::new(),
                }
            }
            Pattern::Type { name, target, span } => {
                let ty = self.lower_type_ref(target);
                let condition = self.emit_temp_from_rvalue(
                    ir::RValue::TypeTest {
                        operand: scrutinee.clone(),
                        ty: ty.clone(),
                    },
                    ir::Type::Bool,
                    Some(*span),
                );
                let bindings = if let Some(binding_name) = name {
                    if binding_name != "_" {
                        vec![PendingBinding {
                            name: binding_name.clone(),
                            ty: ty.clone(),
                            source: PendingBindingSource::RValue(ir::RValue::Cast {
                                operand: scrutinee,
                                ty,
                            }),
                        }]
                    } else {
                        Vec::new()
                    }
                } else {
                    Vec::new()
                };
                PatternPlan {
                    condition,
                    bindings,
                }
            }
            Pattern::Tuple { elements, span } => {
                let mut conditions = Vec::new();
                let mut bindings = Vec::new();
                for (index, element) in elements.iter().enumerate() {
                    let field = self.emit_temp_from_rvalue(
                        ir::RValue::Field {
                            base: scrutinee.clone(),
                            name: format!("_{}", index + 1),
                        },
                        ir::Type::Unknown,
                        Some(*span),
                    );
                    let plan = self.lower_pattern_plan(field, element);
                    conditions.push(plan.condition);
                    bindings.extend(plan.bindings);
                }
                PatternPlan {
                    condition: self.combine_conditions(conditions, *span),
                    bindings,
                }
            }
            Pattern::List {
                elements,
                rest,
                span,
            } => {
                let element_ty = self.list_element_type(&scrutinee);
                let list_ty = ir::Type::Named {
                    name: "Vector".to_string(),
                    args: vec![element_ty.clone()],
                };
                let len = self.emit_temp_from_rvalue(
                    ir::RValue::Call {
                        callee: ir::Callee::Intrinsic(ir::Intrinsic::ListLen),
                        args: vec![scrutinee.clone()],
                        structural: false,
                    },
                    ir::Type::Int,
                    Some(*span),
                );
                let expected_len = ir::Operand::Const(ir::Constant::Int(elements.len() as i64));
                let length_condition = self.emit_temp_from_rvalue(
                    ir::RValue::Binary {
                        op: if rest.is_some() {
                            ir::BinaryOp::GreaterEq
                        } else {
                            ir::BinaryOp::Eq
                        },
                        left: len,
                        right: expected_len,
                    },
                    ir::Type::Bool,
                    Some(*span),
                );
                let mut conditions = vec![length_condition];
                let mut bindings = Vec::new();
                for (index, element) in elements.iter().enumerate() {
                    let item_value = ir::RValue::Call {
                        callee: ir::Callee::Intrinsic(ir::Intrinsic::ListGet),
                        args: vec![
                            scrutinee.clone(),
                            ir::Operand::Const(ir::Constant::Int(index as i64)),
                        ],
                        structural: false,
                    };
                    if let Some(mut deferred) =
                        Self::deferred_list_item_bindings(element, item_value.clone(), &element_ty)
                    {
                        bindings.append(&mut deferred);
                        continue;
                    }
                    let item = self.emit_temp_from_rvalue(
                        item_value,
                        element_ty.clone(),
                        Some(element.span()),
                    );
                    let plan = self.lower_pattern_plan(item, element);
                    conditions.push(plan.condition);
                    bindings.extend(plan.bindings);
                }
                if let Some(rest) = rest {
                    if rest.name != "_" {
                        bindings.push(PendingBinding {
                            name: rest.name.clone(),
                            ty: list_ty,
                            source: PendingBindingSource::RValue(ir::RValue::Call {
                                callee: ir::Callee::Intrinsic(ir::Intrinsic::ListSlice),
                                args: vec![
                                    scrutinee,
                                    ir::Operand::Const(ir::Constant::Int(elements.len() as i64)),
                                ],
                                structural: false,
                            }),
                        });
                    }
                }
                PatternPlan {
                    condition: self.combine_conditions(conditions, *span),
                    bindings,
                }
            }
            Pattern::Record { path, fields, span } => {
                let base_condition = if path.is_empty() {
                    self.bool_const(true)
                } else {
                    let Some(kind) = self.lookup_record_pattern_kind(path, &scrutinee) else {
                        self.add_error(
                            "lower_invariant",
                            "record pattern should be resolved before lowering",
                            *span,
                        );
                        return PatternPlan::always_false();
                    };
                    match &kind {
                        ConstructorPatternKind::EnumCase { case_name, .. } => self
                            .emit_temp_from_rvalue(
                                ir::RValue::Call {
                                    callee: ir::Callee::Intrinsic(ir::Intrinsic::VariantIs(
                                        case_name.clone(),
                                    )),
                                    args: vec![scrutinee.clone()],
                                    structural: false,
                                },
                                ir::Type::Bool,
                                Some(*span),
                            ),
                        ConstructorPatternKind::TypeDestructure { ty, .. } => self
                            .emit_temp_from_rvalue(
                                ir::RValue::TypeTest {
                                    operand: scrutinee.clone(),
                                    ty: ty.clone(),
                                },
                                ir::Type::Bool,
                                Some(*span),
                            ),
                        ConstructorPatternKind::ObjectSingleton { ty } => self
                            .emit_temp_from_rvalue(
                                ir::RValue::TypeTest {
                                    operand: scrutinee.clone(),
                                    ty: ty.clone(),
                                },
                                ir::Type::Bool,
                                Some(*span),
                            ),
                    }
                };
                let mut conditions = vec![base_condition];
                let mut bindings = Vec::new();
                for field in fields {
                    let field_ty = self.record_pattern_field_type(&scrutinee, path, &field.name);
                    if let Pattern::Binding { name, .. } = &field.pattern {
                        if name != "_" {
                            bindings.push(PendingBinding {
                                name: name.clone(),
                                ty: field_ty,
                                source: PendingBindingSource::RValue(ir::RValue::Call {
                                    callee: ir::Callee::Intrinsic(ir::Intrinsic::PatternField(
                                        field.name.clone(),
                                    )),
                                    args: vec![scrutinee.clone()],
                                    structural: false,
                                }),
                            });
                        }
                        continue;
                    }
                    if matches!(field.pattern, Pattern::Wildcard { .. }) {
                        continue;
                    }
                    let value = self.emit_temp_from_rvalue(
                        ir::RValue::Call {
                            callee: ir::Callee::Intrinsic(ir::Intrinsic::PatternField(
                                field.name.clone(),
                            )),
                            args: vec![scrutinee.clone()],
                            structural: false,
                        },
                        field_ty,
                        Some(field.pattern.span()),
                    );
                    let plan = self.lower_pattern_plan(value, &field.pattern);
                    conditions.push(plan.condition);
                    bindings.extend(plan.bindings);
                }
                PatternPlan {
                    condition: self.combine_conditions(conditions, *span),
                    bindings,
                }
            }
            Pattern::Constructor {
                path, args, span, ..
            } => {
                let Some(kind) = self.lookup_constructor_pattern_kind(path, args.len(), &scrutinee)
                else {
                    self.add_error(
                        "lower_invariant",
                        "constructor pattern should be resolved before lowering",
                        *span,
                    );
                    return PatternPlan::always_false();
                };
                let base_condition = match &kind {
                    ConstructorPatternKind::EnumCase { case_name, .. } => self
                        .emit_temp_from_rvalue(
                            ir::RValue::Call {
                                callee: ir::Callee::Intrinsic(ir::Intrinsic::VariantIs(
                                    case_name.clone(),
                                )),
                                args: vec![scrutinee.clone()],
                                structural: false,
                            },
                            ir::Type::Bool,
                            Some(*span),
                        ),
                    ConstructorPatternKind::TypeDestructure { ty, .. } => self
                        .emit_temp_from_rvalue(
                            ir::RValue::TypeTest {
                                operand: scrutinee.clone(),
                                ty: ty.clone(),
                            },
                            ir::Type::Bool,
                            Some(*span),
                        ),
                    ConstructorPatternKind::ObjectSingleton { ty } => self.emit_temp_from_rvalue(
                        ir::RValue::TypeTest {
                            operand: scrutinee.clone(),
                            ty: ty.clone(),
                        },
                        ir::Type::Bool,
                        Some(*span),
                    ),
                };
                let mut conditions = vec![base_condition];
                let mut bindings = Vec::new();
                let (field_names, is_enum_case) = match kind {
                    ConstructorPatternKind::EnumCase { field_names, .. } => (field_names, true),
                    ConstructorPatternKind::TypeDestructure { field_names, .. } => {
                        (field_names, false)
                    }
                    ConstructorPatternKind::ObjectSingleton { .. } => (Vec::new(), false),
                };
                for (index, arg) in args.iter().enumerate() {
                    let field_name = field_names
                        .get(index)
                        .cloned()
                        .unwrap_or_else(|| format!("_{}", index + 1));
                    let field_ty =
                        self.constructor_pattern_field_type(&scrutinee, path, &field_name, index);
                    if let Pattern::Binding { name, .. } = arg {
                        if name != "_" {
                            bindings.push(PendingBinding {
                                name: name.clone(),
                                ty: field_ty,
                                source: PendingBindingSource::RValue(ir::RValue::Call {
                                    callee: ir::Callee::Intrinsic(if is_enum_case {
                                        ir::Intrinsic::VariantField(field_name)
                                    } else {
                                        ir::Intrinsic::PatternField(field_name)
                                    }),
                                    args: vec![scrutinee.clone()],
                                    structural: false,
                                }),
                            });
                        }
                        continue;
                    }
                    if matches!(arg, Pattern::Wildcard { .. }) {
                        continue;
                    }
                    let field = self.emit_temp_from_rvalue(
                        ir::RValue::Call {
                            callee: ir::Callee::Intrinsic(if is_enum_case {
                                ir::Intrinsic::VariantField(field_name)
                            } else {
                                ir::Intrinsic::PatternField(field_name)
                            }),
                            args: vec![scrutinee.clone()],
                            structural: false,
                        },
                        field_ty,
                        Some(arg.span()),
                    );
                    let plan = self.lower_pattern_plan(field, arg);
                    conditions.push(plan.condition);
                    bindings.extend(plan.bindings);
                }
                PatternPlan {
                    condition: self.combine_conditions(conditions, *span),
                    bindings,
                }
            }
        }
    }

    fn deferred_list_item_bindings(
        pattern: &Pattern,
        source: ir::RValue,
        ty: &ir::Type,
    ) -> Option<Vec<PendingBinding>> {
        match pattern {
            Pattern::Wildcard { .. } => Some(Vec::new()),
            Pattern::Binding { name, .. } => Some(if name == "_" {
                Vec::new()
            } else {
                vec![PendingBinding {
                    name: name.clone(),
                    ty: ty.clone(),
                    source: PendingBindingSource::RValue(source),
                }]
            }),
            Pattern::Alias { inner, name, .. } => {
                let mut bindings = Self::deferred_list_item_bindings(inner, source.clone(), ty)?;
                if name != "_" {
                    bindings.push(PendingBinding {
                        name: name.clone(),
                        ty: ty.clone(),
                        source: PendingBindingSource::RValue(source),
                    });
                }
                Some(bindings)
            }
            _ => None,
        }
    }

    fn whole_pattern_binding_type(&self, pattern: &Pattern, scrutinee: &ir::Operand) -> ir::Type {
        match pattern {
            Pattern::Alias { inner, .. } => self.whole_pattern_binding_type(inner, scrutinee),
            Pattern::Type { target, .. } => self.lower_type_ref(target),
            Pattern::Record { path, .. } if !path.is_empty() => {
                match self.lookup_record_pattern_kind(path, scrutinee) {
                    Some(ConstructorPatternKind::EnumCase { case_name, .. }) => self
                        .enum_case_view_type(scrutinee, &case_name)
                        .unwrap_or_else(|| {
                            self.operand_type(scrutinee).unwrap_or(ir::Type::Unknown)
                        }),
                    Some(ConstructorPatternKind::TypeDestructure { ty, .. }) => ty,
                    _ => self.operand_type(scrutinee).unwrap_or(ir::Type::Unknown),
                }
            }
            Pattern::Constructor { path, args, .. } => {
                match self.lookup_constructor_pattern_kind(path, args.len(), scrutinee) {
                    Some(ConstructorPatternKind::EnumCase { case_name, .. }) => self
                        .enum_case_view_type(scrutinee, &case_name)
                        .unwrap_or_else(|| {
                            self.operand_type(scrutinee).unwrap_or(ir::Type::Unknown)
                        }),
                    Some(ConstructorPatternKind::TypeDestructure { ty, .. })
                    | Some(ConstructorPatternKind::ObjectSingleton { ty }) => ty,
                    _ => self.operand_type(scrutinee).unwrap_or(ir::Type::Unknown),
                }
            }
            _ => self.operand_type(scrutinee).unwrap_or(ir::Type::Unknown),
        }
    }

    fn enum_case_view_type(&self, scrutinee: &ir::Operand, case_name: &str) -> Option<ir::Type> {
        let ir::Type::Named { name, args } = self.operand_type(scrutinee)? else {
            return None;
        };
        Some(ir::Type::Named {
            name: enum_case_view_name(&name, case_name),
            args,
        })
    }

    fn constructor_pattern_field_type(
        &self,
        scrutinee: &ir::Operand,
        path: &[String],
        field_name: &str,
        index: usize,
    ) -> ir::Type {
        let Some(scrutinee_ty) = self.operand_type(scrutinee) else {
            return ir::Type::Unknown;
        };
        if path.is_empty() {
            return match &scrutinee_ty {
                ir::Type::Record(fields) => fields
                    .iter()
                    .find(|field| field.name == field_name)
                    .map(|field| field.ty.clone())
                    .unwrap_or(ir::Type::Unknown),
                ir::Type::Named { name, .. } => self
                    .program
                    .types
                    .iter()
                    .find(|ty| ty.name == *name)
                    .and_then(|ty| ty.fields.iter().find(|field| field.name == field_name))
                    .map(|field| field.ty.clone())
                    .unwrap_or(ir::Type::Unknown),
                _ => ir::Type::Unknown,
            };
        }
        if let Some(ty) = self.enum_case_field_type(&scrutinee_ty, path, field_name, index) {
            return ty;
        }
        let Some(type_name) = self.canonical_declared_type_path(path) else {
            return ir::Type::Unknown;
        };
        let Some(ty) = self.program.types.iter().find(|ty| {
            ty.name == type_name && matches!(ty.kind, ast::TypeKind::Class | ast::TypeKind::Record)
        }) else {
            return ir::Type::Unknown;
        };
        let subst = match &scrutinee_ty {
            ir::Type::Named { name, args } if name == &ty.name => ir_type_subst(ty, args),
            _ => HashMap::new(),
        };
        ty.fields
            .iter()
            .find(|field| field.visibility != ast::Visibility::Private && field.name == field_name)
            .map(|field| substitute_ir_type(&field.ty, &subst))
            .unwrap_or(ir::Type::Unknown)
    }

    fn record_pattern_field_type(
        &self,
        scrutinee: &ir::Operand,
        path: &[String],
        field_name: &str,
    ) -> ir::Type {
        let Some(scrutinee_ty) = self.operand_type(scrutinee) else {
            return ir::Type::Unknown;
        };
        if path.is_empty() {
            return match &scrutinee_ty {
                ir::Type::Record(fields) => fields
                    .iter()
                    .find(|field| field.name == field_name)
                    .map(|field| field.ty.clone())
                    .unwrap_or(ir::Type::Unknown),
                ir::Type::Named { name, .. } => self
                    .program
                    .types
                    .iter()
                    .find(|ty| ty.name == *name)
                    .and_then(|ty| ty.fields.iter().find(|field| field.name == field_name))
                    .map(|field| field.ty.clone())
                    .unwrap_or(ir::Type::Unknown),
                _ => ir::Type::Unknown,
            };
        }
        if let Some(ty) = self.enum_case_field_type(&scrutinee_ty, path, field_name, 0) {
            return ty;
        }
        let Some(type_name) = self.canonical_declared_type_path(path) else {
            return ir::Type::Unknown;
        };
        let Some(ty) = self.program.types.iter().find(|ty| {
            ty.name == type_name && matches!(ty.kind, ast::TypeKind::Class | ast::TypeKind::Record)
        }) else {
            return ir::Type::Unknown;
        };
        ty.fields
            .iter()
            .find(|field| field.name == field_name)
            .map(|field| field.ty.clone())
            .unwrap_or(ir::Type::Unknown)
    }

    fn list_element_type(&self, scrutinee: &ir::Operand) -> ir::Type {
        match self.operand_type(scrutinee) {
            Some(ir::Type::Named { name, args }) if name == "Vector" && args.len() == 1 => {
                args.into_iter().next().unwrap_or(ir::Type::Unknown)
            }
            _ => ir::Type::Unknown,
        }
    }

    fn enum_case_field_type(
        &self,
        scrutinee_ty: &ir::Type,
        path: &[String],
        field_name: &str,
        index: usize,
    ) -> Option<ir::Type> {
        let case_name = path.last()?;
        let ir::Type::Named { name, args } = scrutinee_ty else {
            return None;
        };

        match (name.as_str(), case_name.as_str(), field_name) {
            ("Option", "Some", "value") if args.len() == 1 => return args.first().cloned(),
            ("Result", "Ok", "value") if args.len() == 2 => return args.first().cloned(),
            ("Result", "Err", "error") if args.len() == 2 => return args.get(1).cloned(),
            ("Either", "Left", "value") if args.len() == 2 => return args.first().cloned(),
            ("Either", "Right", "value") if args.len() == 2 => return args.get(1).cloned(),
            (
                "FileError",
                "NotFound" | "AccessDenied" | "Closed" | "InvalidEncoding" | "IoFailure",
                "path",
            ) => return Some(ir::Type::Str),
            ("FileError", "InvalidEncoding", "offset") => return Some(ir::Type::Int),
            ("FileError", "IoFailure", "operation" | "message") => {
                return Some(ir::Type::Str);
            }
            _ => {}
        }

        let ty = self
            .program
            .types
            .iter()
            .find(|ty| ty.kind == ast::TypeKind::Enum && ty.name == *name)?;
        let case = ty.enum_cases.iter().find(|case| case.name == *case_name)?;
        let subst = ir_type_subst(ty, args);
        case.fields
            .iter()
            .find(|field| field.name == field_name)
            .or_else(|| case.fields.get(index))
            .map(|field| substitute_ir_type(&field.ty, &subst))
    }

    fn lower_pattern_literal_expr(&mut self, expr: &ast::Expr) -> ir::Operand {
        let expr = desugar::desugar_expr(expr);
        self.lower_expr(&expr)
    }

    fn lookup_constructor_pattern_kind(
        &self,
        path: &[String],
        arity: usize,
        scrutinee: &ir::Operand,
    ) -> Option<ConstructorPatternKind> {
        if let Some(field_names) = self.lookup_case_fields(path, arity, scrutinee) {
            return Some(ConstructorPatternKind::EnumCase {
                case_name: path.last().cloned().unwrap_or_default(),
                field_names,
            });
        }
        if let Some((ty, field_names)) = self.lookup_destructured_type_fields(path, arity) {
            return Some(ConstructorPatternKind::TypeDestructure { ty, field_names });
        }
        if arity == 0 {
            if let Some(ty) = self.lookup_object_pattern_type(path) {
                return Some(ConstructorPatternKind::ObjectSingleton { ty });
            }
        }
        None
    }

    fn lookup_object_pattern_type(&self, path: &[String]) -> Option<ir::Type> {
        let type_name = self.canonical_declared_type_path(path)?;
        self.program
            .types
            .iter()
            .any(|ty| ty.kind == ast::TypeKind::Object && ty.name == type_name)
            .then(|| ir::Type::named(type_name))
    }

    fn lookup_record_pattern_kind(
        &self,
        path: &[String],
        scrutinee: &ir::Operand,
    ) -> Option<ConstructorPatternKind> {
        if let Some(field_names) = self.lookup_case_all_fields(path, scrutinee) {
            return Some(ConstructorPatternKind::EnumCase {
                case_name: path.last().cloned().unwrap_or_default(),
                field_names,
            });
        }
        self.lookup_destructured_type_all_fields(path)
            .map(|(ty, field_names)| ConstructorPatternKind::TypeDestructure { ty, field_names })
    }

    fn lookup_case_all_fields(
        &self,
        path: &[String],
        scrutinee: &ir::Operand,
    ) -> Option<Vec<String>> {
        let case_name = path.last()?;
        if path.len() >= 2 {
            let type_name = &path[path.len() - 2];
            if let Some(case) = self
                .program
                .types
                .iter()
                .filter(|ty| ty.kind == ast::TypeKind::Enum && ty.name == *type_name)
                .flat_map(|ty| ty.enum_cases.iter())
                .find(|case| case.name == *case_name)
            {
                return Some(case.fields.iter().map(|field| field.name.clone()).collect());
            }
        }
        if let Some(fields) = core_pattern_case_fields(path, self.operand_type(scrutinee).as_ref())
        {
            return Some(fields);
        }
        let ast_matches = self
            .program
            .types
            .iter()
            .filter(|ty| ty.kind == ast::TypeKind::Enum)
            .flat_map(|ty| ty.enum_cases.iter())
            .filter(|case| case.name == *case_name)
            .map(|case| {
                case.fields
                    .iter()
                    .map(|field| field.name.clone())
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        if ast_matches.len() == 1 {
            return ast_matches.into_iter().next();
        }
        if let Some(first) = ast_matches.first() {
            if ast_matches.iter().all(|fields| fields == first) {
                return Some(first.clone());
            }
        }
        let matches = self
            .case_fields
            .iter()
            .filter_map(|(key, fields)| {
                key.ends_with(&format!(".{case_name}"))
                    .then_some(fields.clone())
            })
            .collect::<Vec<_>>();
        if matches.len() == 1 {
            return matches.into_iter().next();
        }
        matches
            .first()
            .filter(|first| matches.iter().all(|fields| fields == *first))
            .cloned()
    }

    fn lookup_case_fields(
        &self,
        path: &[String],
        arity: usize,
        scrutinee: &ir::Operand,
    ) -> Option<Vec<String>> {
        let case_name = path.last()?;
        if path.len() >= 2 {
            let type_name = &path[path.len() - 2];
            if let Some((ty, case)) = self
                .program
                .types
                .iter()
                .filter(|ty| ty.kind == ast::TypeKind::Enum && ty.name == *type_name)
                .find_map(|ty| {
                    ty.enum_cases
                        .iter()
                        .find(|case| case.name == *case_name)
                        .map(|case| (ty, case))
                })
            {
                if let Some(fields) = enum_case_pattern_fields(ty, case, arity) {
                    return Some(fields);
                }
            }
        }
        if let Some(fields) = core_pattern_case_fields(path, self.operand_type(scrutinee).as_ref())
            && fields.len() == arity
        {
            return Some(fields);
        }

        let ast_matches = self
            .program
            .types
            .iter()
            .filter(|ty| ty.kind == ast::TypeKind::Enum)
            .filter_map(|ty| {
                ty.enum_cases
                    .iter()
                    .find(|case| case.name == *case_name)
                    .and_then(|case| enum_case_pattern_fields(ty, case, arity))
            })
            .collect::<Vec<_>>();
        if ast_matches.len() == 1 {
            return ast_matches.into_iter().next();
        }
        if let Some(first) = ast_matches.first() {
            if ast_matches.iter().all(|fields| fields == first) {
                return Some(first.clone());
            }
        }

        if path.len() >= 2 {
            let key = format!("{}.{}", path[path.len() - 2], case_name);
            if let Some(fields) = self.case_fields.get(&key) {
                if fields.len() == arity {
                    return Some(fields.clone());
                }
            }
        }

        let matches = self
            .case_fields
            .iter()
            .filter_map(|(key, fields)| {
                if key.ends_with(&format!(".{case_name}")) && fields.len() == arity {
                    Some(fields.clone())
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        if matches.len() == 1 {
            return matches.into_iter().next();
        }
        if let Some(first) = matches.first() {
            if matches.iter().all(|fields| fields == first) {
                return Some(first.clone());
            }
        }
        None
    }

    fn lookup_destructured_type_fields(
        &self,
        path: &[String],
        arity: usize,
    ) -> Option<(ir::Type, Vec<String>)> {
        let type_name = self.canonical_declared_type_path(path)?;
        let ty = self.program.types.iter().find(|ty| {
            ty.name == type_name && matches!(ty.kind, ast::TypeKind::Class | ast::TypeKind::Record)
        })?;
        let visible_fields = ty
            .fields
            .iter()
            .filter(|field| field.visibility != ast::Visibility::Private)
            .collect::<Vec<_>>();
        if visible_fields.len() != arity {
            return None;
        }
        Some((
            ir::Type::named(type_name),
            visible_fields
                .iter()
                .map(|field| field.name.clone())
                .collect(),
        ))
    }

    fn lookup_destructured_type_all_fields(
        &self,
        path: &[String],
    ) -> Option<(ir::Type, Vec<String>)> {
        let type_name = self.canonical_declared_type_path(path)?;
        let ty = self.program.types.iter().find(|ty| {
            ty.name == type_name && matches!(ty.kind, ast::TypeKind::Class | ast::TypeKind::Record)
        })?;
        Some((
            ir::Type::named(type_name),
            ty.fields
                .iter()
                .filter(|field| field.visibility != ast::Visibility::Private)
                .map(|field| field.name.clone())
                .collect(),
        ))
    }

    fn apply_pending_bindings(&mut self, bindings: Vec<PendingBinding>) -> Vec<AppliedBinding> {
        let mut applied = Vec::with_capacity(bindings.len());
        for binding in bindings {
            let name = binding.name.clone();
            let ty = binding.ty.clone();
            let local_id = self.add_local(name.clone(), binding.ty, false, ir::LocalKind::Binding);
            self.current_scope().insert(name.clone(), local_id);
            let value = match binding.source {
                PendingBindingSource::Operand(value) => ir::RValue::Use(value),
                PendingBindingSource::RValue(value) => value,
            };
            self.push_statement(ir::Statement {
                span: None,
                kind: ir::StatementKind::Assign {
                    target: ir::Place::Local(local_id),
                    value,
                },
            });
            applied.push(AppliedBinding {
                name,
                ty,
                local: local_id,
            });
        }
        applied
    }

    fn emit_temp_from_rvalue(
        &mut self,
        value: ir::RValue,
        ty: ir::Type,
        span: Option<Span>,
    ) -> ir::Operand {
        let temp = self.add_temp(ty);
        self.push_statement(ir::Statement {
            span,
            kind: ir::StatementKind::Assign {
                target: ir::Place::Local(temp),
                value,
            },
        });
        ir::Operand::Copy(Box::new(ir::Place::Local(temp)))
    }

    fn force_lazy_value(
        &mut self,
        thunk: ir::Operand,
        value_ty: ir::Type,
        span: Span,
    ) -> ir::Operand {
        self.emit_temp_from_rvalue(
            ir::RValue::Call {
                callee: ir::Callee::Indirect(thunk),
                args: Vec::new(),
                structural: false,
            },
            value_ty,
            Some(span),
        )
    }

    fn combine_conditions(&mut self, conditions: Vec<ir::Operand>, span: Span) -> ir::Operand {
        let mut iter = conditions.into_iter();
        let Some(first) = iter.next() else {
            return self.bool_const(true);
        };
        iter.fold(first, |left, right| {
            self.emit_temp_from_rvalue(
                ir::RValue::Binary {
                    op: ir::BinaryOp::And,
                    left,
                    right,
                },
                ir::Type::Bool,
                Some(span),
            )
        })
    }

    fn bool_const(&self, value: bool) -> ir::Operand {
        ir::Operand::Const(ir::Constant::Bool(value))
    }

    fn lower_expr_with_expected(
        &mut self,
        expr: &Expr,
        expected: Option<&ir::Type>,
    ) -> ir::Operand {
        self.retain_source_expr(expr, expected);
        if let Some(value) = explicit_any_widening_value(expr) {
            return self.lower_expr(value);
        }
        let Some(expected) = expected else {
            return self.lower_expr(expr);
        };
        if self.current_block.is_none() {
            return ir::Operand::Const(ir::Constant::Unit);
        }
        match expr {
            Expr::ListLiteral { items, span }
                if items.is_empty()
                    && matches!(
                        expected,
                        ir::Type::Named { name, args } if name == "Map" && args.len() == 2
                    ) =>
            {
                self.emit_temp_from_rvalue(
                    ir::RValue::Call {
                        callee: ir::Callee::Named {
                            path: vec!["Map".to_string()],
                        },
                        args: Vec::new(),
                        structural: false,
                    },
                    expected.clone(),
                    Some(*span),
                )
            }
            Expr::ListLiteral { items, span }
                if list_literal_has_spread(items)
                    && self.spread_only_literal_is_map(items, Some(expected)) =>
            {
                self.lower_spread_map_literal(items, *span, Some(expected))
            }
            Expr::ListLiteral { items, span } if list_literal_has_spread(items) => {
                self.lower_spread_list_literal(items, *span)
            }
            Expr::Match { value, cases, span } => {
                self.lower_match_expr_with_expected(value, cases, *span, Some(expected))
            }
            Expr::Block { body, .. } => self
                .lower_block_value_with_expected(body, Some(expected))
                .unwrap_or(ir::Operand::Const(ir::Constant::Unit)),
            Expr::If {
                condition_clauses,
                then_block,
                else_branch,
                span,
            } => self.lower_if_expr(
                condition_clauses,
                then_block,
                else_branch,
                *span,
                Some(expected),
            ),
            Expr::ExtractOr {
                value,
                fallback,
                span,
            } => self.lower_extract_or_expr(value, fallback, *span, Some(expected)),
            Expr::Call { .. }
            | Expr::ContextualNew { .. }
            | Expr::RecordLiteral { .. }
            | Expr::Lambda { .. }
            | Expr::AnonymousObject { .. }
            | Expr::Unary {
                op: ast::UnaryOp::OptionWrap,
                ..
            } => self.lower_expr_from_rvalue_with_expected(expr, Some(expected)),
            Expr::Identifier { .. } | Expr::Member { .. }
                if matches!(expected, ir::Type::Function { .. })
                    && self.is_callable_reference_expr(expr) =>
            {
                self.lower_expr_from_rvalue_with_expected(expr, Some(expected))
            }
            _ => self.lower_expr(expr),
        }
    }

    fn lower_expr(&mut self, expr: &Expr) -> ir::Operand {
        self.retain_source_expr(expr, None);
        if self.current_block.is_none() {
            return ir::Operand::Const(ir::Constant::Unit);
        }
        if let Some(value) = explicit_any_widening_value(expr) {
            return self.lower_expr(value);
        }
        if let Some(reference_ty) = self.callable_reference_type(expr) {
            return self.lower_expr_from_rvalue_with_expected(expr, Some(&reference_ty));
        }
        match expr {
            Expr::Identifier { name, span } => {
                if let Some(value) = self.lookup_scoped_or_captured_value(name) {
                    if let Some(value_ty) = self.lazy_values.get(name).cloned() {
                        return self.force_lazy_value(value, value_ty, *span);
                    }
                    return value;
                }
                {
                    if let Some(place) = self.lookup_implicit_field_place(name) {
                        return ir::Operand::Copy(Box::new(place));
                    }
                    if let Some((getter, subst)) = self
                        .lookup_scoped_type("this")
                        .or_else(|| {
                            self.capture_sources
                                .get("this")
                                .map(|source| source.ty.clone())
                        })
                        .and_then(|this_ty| self.getter_function_for_type(&this_ty, name))
                    {
                        let return_ty = self
                            .program
                            .function(getter)
                            .map(|function| substitute_ir_type(&function.return_ty, &subst))
                            .unwrap_or(ir::Type::Unknown);
                        let receiver = self
                            .lookup_scoped_or_captured_value("this")
                            .unwrap_or(ir::Operand::Const(ir::Constant::Unit));
                        return self.emit_temp_from_rvalue(
                            ir::RValue::Call {
                                callee: ir::Callee::Method {
                                    receiver,
                                    method: name.clone(),
                                },
                                args: Vec::new(),
                                structural: false,
                            },
                            return_ty,
                            Some(*span),
                        );
                    }
                    if let Some(place) = self.lookup_global_place(name) {
                        return ir::Operand::Copy(Box::new(place));
                    }
                    let path = vec![name.clone()];
                    if let Some(canonical) = self.canonical_declared_type_path(&path)
                        && self
                            .program
                            .types
                            .iter()
                            .any(|ty| ty.name == canonical && ty.kind == ast::TypeKind::Object)
                    {
                        return self.emit_temp_from_rvalue(
                            ir::RValue::NamedValue {
                                path: vec![canonical],
                            },
                            ir::Type::Unknown,
                            Some(*span),
                        );
                    }
                    if is_named_runtime_value_path(self.program, &path) {
                        let path = unique_bare_enum_case_owner(self.program, name)
                            .map(|owner| vec![owner.to_string(), name.clone()])
                            .unwrap_or(path);
                        return self.emit_temp_from_rvalue(
                            ir::RValue::NamedValue { path },
                            ir::Type::Unknown,
                            Some(*span),
                        );
                    }
                    self.add_error(
                        "lower_invariant",
                        format!("value '{}' should resolve before lowering", name),
                        *span,
                    );
                    ir::Operand::Const(ir::Constant::Unit)
                }
            }
            Expr::Integer { raw, .. } => raw
                .parse::<i64>()
                .map(ir::Constant::Int)
                .map(ir::Operand::Const)
                .unwrap_or(ir::Operand::Const(ir::Constant::Int(0))),
            Expr::Float { raw, .. } => raw
                .parse::<f64>()
                .map(ir::Constant::Float)
                .map(ir::Operand::Const)
                .unwrap_or(ir::Operand::Const(ir::Constant::Float(0.0))),
            Expr::String { raw, .. } => ir::Operand::Const(ir::Constant::String(raw.clone())),
            Expr::Bool { value, .. } => ir::Operand::Const(ir::Constant::Bool(*value)),
            Expr::Unit { .. } => ir::Operand::Const(ir::Constant::Unit),
            Expr::Binary {
                left,
                op: AstBinaryOp::And,
                right,
                span,
            } => self.lower_logical_expr(left, AstBinaryOp::And, right, *span),
            Expr::Binary {
                left,
                op: AstBinaryOp::Or,
                right,
                span,
            } => self.lower_logical_expr(left, AstBinaryOp::Or, right, *span),
            Expr::If {
                condition_clauses,
                then_block,
                else_branch,
                span,
            } => self.lower_if_expr(condition_clauses, then_block, else_branch, *span, None),
            Expr::Try { value, span } => self.lower_try_expr(value, *span),
            Expr::ExtractOr {
                value,
                fallback,
                span,
            } => self.lower_extract_or_expr(value, fallback, *span, None),
            Expr::Return { value, span } => self.lower_return_control_expr(value.as_deref(), *span),
            Expr::Break { span } => self.lower_break_control_expr(*span),
            Expr::Continue { span } => self.lower_continue_control_expr(*span),
            Expr::Block { body, .. } => self.lower_block_expr(body),
            Expr::Match { value, cases, span } => self.lower_match_expr(value, cases, *span),
            Expr::ForYield {
                bindings,
                yield_body,
                span,
            } => self.lower_for_yield_expr(bindings, yield_body, *span),
            Expr::ListLiteral { items, span } if self.spread_only_literal_is_map(items, None) => {
                self.lower_spread_map_literal(items, *span, None)
            }
            Expr::ListLiteral { items, span } if list_literal_has_spread(items) => {
                self.lower_spread_list_literal(items, *span)
            }
            Expr::ListLiteral { .. }
            | Expr::TupleLiteral { .. }
            | Expr::RecordLiteral { .. }
            | Expr::AnonymousObject { .. }
            | Expr::Unary { .. }
            | Expr::Binary { .. }
            | Expr::Call { .. }
            | Expr::ContextualNew { .. }
            | Expr::Member { .. }
            | Expr::Index { .. }
            | Expr::RecordUpdate { .. }
            | Expr::Is { .. }
            | Expr::TypeOf { .. }
            | Expr::Lambda { .. } => self.lower_expr_from_rvalue(expr),
            Expr::Spread { value, .. } => self.lower_expr(value),
            Expr::Placeholder { span } => {
                self.add_error(
                    "lower_invariant",
                    "placeholder '_' cannot appear as an expression; use an explicit lambda parameter slot",
                    *span,
                );
                ir::Operand::Const(ir::Constant::Unit)
            }
        }
    }

    fn retain_source_expr(&mut self, expr: &Expr, expected: Option<&ir::Type>) {
        let span = expr.span();
        let inferred = self.infer_expr_type(expr);
        if let Some(existing) = self
            .program
            .source_exprs
            .iter_mut()
            .find(|source| source.function == self.function_id && source.span == span)
        {
            if matches!(existing.ty, ir::Type::Unknown) && !matches!(inferred, ir::Type::Unknown) {
                existing.ty = inferred;
            }
            if existing.expected.is_none() {
                existing.expected = expected.cloned();
            }
            return;
        }
        self.program.source_exprs.push(ir::SourceExpr {
            function: self.function_id,
            span,
            ty: inferred,
            expected: expected.cloned(),
        });
    }

    fn lower_expr_from_rvalue(&mut self, expr: &Expr) -> ir::Operand {
        self.lower_expr_from_rvalue_with_expected(expr, None)
    }

    fn lower_spread_list_literal(&mut self, items: &[Expr], span: Span) -> ir::Operand {
        let list = self.add_temp(ir::Type::list(ir::Type::Unknown));
        self.push_statement(ir::Statement {
            span: Some(span),
            kind: ir::StatementKind::Assign {
                target: ir::Place::Local(list),
                value: ir::RValue::List(Vec::new()),
            },
        });

        for item in items {
            let (intrinsic, value, item_span) = match item {
                Expr::Spread { value, span, .. } => {
                    (ir::Intrinsic::ListExtend, self.lower_expr(value), *span)
                }
                _ => (
                    ir::Intrinsic::ListAppend,
                    self.lower_expr(item),
                    item.span(),
                ),
            };
            self.push_statement(ir::Statement {
                span: Some(item_span),
                kind: ir::StatementKind::Assign {
                    target: ir::Place::Local(list),
                    value: ir::RValue::Call {
                        callee: ir::Callee::Intrinsic(intrinsic),
                        args: vec![ir::Operand::Copy(Box::new(ir::Place::Local(list))), value],
                        structural: false,
                    },
                },
            });
        }

        ir::Operand::Copy(Box::new(ir::Place::Local(list)))
    }

    fn lower_spread_map_literal(
        &mut self,
        items: &[Expr],
        span: Span,
        expected: Option<&ir::Type>,
    ) -> ir::Operand {
        let map_ty = expected
            .cloned()
            .unwrap_or_else(|| self.infer_spread_map_type(items));
        let args = items
            .iter()
            .filter_map(|item| match item {
                Expr::Spread { value, .. } => Some(self.lower_expr(value)),
                _ => None,
            })
            .collect();
        let map = self.add_temp(map_ty);
        self.push_statement(ir::Statement {
            span: Some(span),
            kind: ir::StatementKind::Assign {
                target: ir::Place::Local(map),
                value: ir::RValue::Call {
                    callee: ir::Callee::Named {
                        path: vec!["Map".to_string()],
                    },
                    args,
                    structural: false,
                },
            },
        });
        ir::Operand::Copy(Box::new(ir::Place::Local(map)))
    }

    fn lower_expr_from_rvalue_with_expected(
        &mut self,
        expr: &Expr,
        expected: Option<&ir::Type>,
    ) -> ir::Operand {
        let ty = expected
            .cloned()
            .unwrap_or_else(|| inferred_storage_type(self.infer_expr_type(expr)));
        let rvalue = if expected.is_some() {
            self.lower_rvalue_with_expected(expr, expected)
        } else {
            self.lower_rvalue(expr)
        }
        .expect("rvalue-backed expression should lower to an rvalue");
        let temp = self.add_temp(ty);
        self.push_statement(ir::Statement {
            span: Some(expr.span()),
            kind: ir::StatementKind::Assign {
                target: ir::Place::Local(temp),
                value: rvalue,
            },
        });
        ir::Operand::Copy(Box::new(ir::Place::Local(temp)))
    }

    fn lower_try_expr(&mut self, value: &Expr, span: Span) -> ir::Operand {
        let source_ty = self.infer_expr_type(value);
        let source = self.lower_expr_with_expected(value, Some(&source_ty));
        let source_local = self.add_temp(ir::Type::Unknown);
        self.push_statement(ir::Statement {
            span: Some(value.span()),
            kind: ir::StatementKind::Assign {
                target: ir::Place::Local(source_local),
                value: ir::RValue::Use(source),
            },
        });

        let success_block = self.add_block();
        let failure_block = self.add_block();
        let join_block = self.add_block();
        let result = self.add_temp(ir::Type::Unknown);

        let present = self.emit_temp_from_rvalue(
            ir::RValue::Call {
                callee: ir::Callee::Method {
                    receiver: ir::Operand::Copy(Box::new(ir::Place::Local(source_local))),
                    method: "isSuccess".to_string(),
                },
                args: Vec::new(),
                structural: false,
            },
            ir::Type::Bool,
            Some(span),
        );
        self.terminate(ir::Terminator {
            span: Some(span),
            kind: ir::TerminatorKind::Branch {
                condition: present,
                then_block: success_block,
                else_block: failure_block,
            },
        });

        self.current_block = Some(success_block);
        let inner = self.emit_temp_from_rvalue(
            ir::RValue::Call {
                callee: ir::Callee::Intrinsic(ir::Intrinsic::ExtractSuccessValue),
                args: vec![ir::Operand::Copy(Box::new(ir::Place::Local(source_local)))],
                structural: false,
            },
            ir::Type::Unknown,
            Some(span),
        );
        self.push_statement(ir::Statement {
            span: Some(span),
            kind: ir::StatementKind::Assign {
                target: ir::Place::Local(result),
                value: ir::RValue::Use(inner),
            },
        });
        if self.current_block.is_some() {
            self.terminate(ir::Terminator::goto(join_block));
        }

        self.current_block = Some(failure_block);
        let failure = ir::Operand::Copy(Box::new(ir::Place::Local(source_local)));
        if self.current_block.is_some() {
            self.terminate(ir::Terminator::ret(Some(failure)));
        }

        let join_used = self.block_has_predecessor(join_block);
        self.current_block = if join_used { Some(join_block) } else { None };
        ir::Operand::Copy(Box::new(ir::Place::Local(result)))
    }

    fn lower_extract_or_expr(
        &mut self,
        value: &Expr,
        fallback: &Expr,
        span: Span,
        expected: Option<&ir::Type>,
    ) -> ir::Operand {
        let source_ty = self.infer_expr_type(value);
        let mut success_ty = unwrap_lifted_ir_type(&source_ty)
            .map(|(_, inner)| inner)
            .unwrap_or(ir::Type::Unknown);
        let result_ty = expected.cloned().unwrap_or_else(|| {
            if success_ty == ir::Type::Unknown {
                self.infer_expr_type(fallback)
            } else {
                success_ty.clone()
            }
        });
        if success_ty == ir::Type::Unknown {
            success_ty = result_ty.clone();
        }
        let source = self.lower_expr_with_expected(value, Some(&source_ty));
        let source_local = self.add_temp(source_ty);
        self.push_statement(ir::Statement {
            span: Some(value.span()),
            kind: ir::StatementKind::Assign {
                target: ir::Place::Local(source_local),
                value: ir::RValue::Use(source),
            },
        });

        let success_block = self.add_block();
        let failure_block = self.add_block();
        let join_block = self.add_block();
        let result = self.add_temp(result_ty.clone());

        let present = self.emit_temp_from_rvalue(
            ir::RValue::Call {
                callee: ir::Callee::Intrinsic(ir::Intrinsic::ExtractSuccessIsSet),
                args: vec![ir::Operand::Copy(Box::new(ir::Place::Local(source_local)))],
                structural: false,
            },
            ir::Type::Bool,
            Some(span),
        );
        self.terminate(ir::Terminator {
            span: Some(span),
            kind: ir::TerminatorKind::Branch {
                condition: present,
                then_block: success_block,
                else_block: failure_block,
            },
        });

        self.current_block = Some(success_block);
        let extracted = self.emit_temp_from_rvalue(
            ir::RValue::Call {
                callee: ir::Callee::Intrinsic(ir::Intrinsic::ExtractSuccessValue),
                args: vec![ir::Operand::Copy(Box::new(ir::Place::Local(source_local)))],
                structural: false,
            },
            success_ty,
            Some(span),
        );
        self.push_statement(ir::Statement {
            span: Some(span),
            kind: ir::StatementKind::Assign {
                target: ir::Place::Local(result),
                value: ir::RValue::Use(extracted),
            },
        });
        if self.current_block.is_some() {
            self.terminate(ir::Terminator::goto(join_block));
        }

        self.current_block = Some(failure_block);
        let fallback_value = self.lower_expr_with_expected(fallback, Some(&result_ty));
        if self.current_block.is_some() {
            self.push_statement(ir::Statement {
                span: Some(fallback.span()),
                kind: ir::StatementKind::Assign {
                    target: ir::Place::Local(result),
                    value: ir::RValue::Use(fallback_value),
                },
            });
            self.terminate(ir::Terminator::goto(join_block));
        }

        let join_used = self.block_has_predecessor(join_block);
        self.current_block = if join_used { Some(join_block) } else { None };
        ir::Operand::Copy(Box::new(ir::Place::Local(result)))
    }

    fn lower_return_control_expr(&mut self, value: Option<&Expr>, span: Span) -> ir::Operand {
        let return_ty = self.function().return_ty.clone();
        let value = value.map(|expr| self.lower_expr_with_expected(expr, Some(&return_ty)));
        self.terminate(ir::Terminator {
            span: Some(span),
            kind: ir::TerminatorKind::Return(value),
        });
        ir::Operand::Const(ir::Constant::Unit)
    }

    fn lower_break_control_expr(&mut self, span: Span) -> ir::Operand {
        if let Some(exit) = self.loop_exits.last().copied() {
            self.terminate(ir::Terminator {
                span: Some(span),
                kind: ir::TerminatorKind::Goto(exit),
            });
        } else {
            self.invariant("break should be rejected before lowering", span);
        }
        ir::Operand::Const(ir::Constant::Unit)
    }

    fn lower_continue_control_expr(&mut self, span: Span) -> ir::Operand {
        if let Some(target) = self.loop_continues.last().copied() {
            self.terminate(ir::Terminator {
                span: Some(span),
                kind: ir::TerminatorKind::Goto(target),
            });
        } else {
            self.invariant("continue should be rejected before lowering", span);
        }
        ir::Operand::Const(ir::Constant::Unit)
    }

    fn infer_expr_type(&self, expr: &Expr) -> ir::Type {
        self.infer_expr_type_with_overrides(expr, &[])
    }

    fn infer_assignment_target_type(&self, expr: &Expr) -> ir::Type {
        let Expr::Index { receiver, .. } = expr else {
            return self.infer_expr_type(expr);
        };
        let receiver_ty = self.infer_expr_type(receiver);
        match receiver_ty {
            ir::Type::Named { name, args }
                if (name == "Vector" || name == "Array") && args.len() == 1 =>
            {
                args[0].clone()
            }
            ir::Type::Named { name, args } if name == "Map" && args.len() == 2 => args[1].clone(),
            other => index_result_ir_type(&other),
        }
    }

    fn infer_expr_type_with_overrides(
        &self,
        expr: &Expr,
        overrides: &[(String, ir::Type)],
    ) -> ir::Type {
        match expr {
            Expr::Identifier { name, .. } => self
                .lookup_override_type(name, overrides)
                .or_else(|| self.lookup_scoped_type(name))
                .or_else(|| self.lookup_implicit_field_type(name))
                .or_else(|| self.lookup_implicit_getter_type(name))
                .or_else(|| self.lookup_global_type(name))
                .or_else(|| self.lookup_function_type(name))
                .or_else(|| self.lookup_bare_enum_case_type(name))
                .or_else(|| self.lookup_declared_type_value(name))
                .unwrap_or(ir::Type::Unknown),
            Expr::Integer { .. } => ir::Type::Int,
            Expr::Float { .. } => ir::Type::Float,
            Expr::String { .. } => ir::Type::Str,
            Expr::Bool { .. } => ir::Type::Bool,
            Expr::Unit { .. } => ir::Type::Unit,
            Expr::ListLiteral { items, .. } => {
                if self.spread_only_literal_is_map(items, None) {
                    return self.infer_spread_map_type(items);
                }
                let item_ty = items
                    .iter()
                    .map(|item| {
                        if let Expr::Spread { value, .. } = item {
                            let spread_ty = self.infer_expr_type_with_overrides(value, overrides);
                            known_iterable_ir_item_type(&spread_ty).unwrap_or(ir::Type::Unknown)
                        } else {
                            self.infer_expr_type_with_overrides(item, overrides)
                        }
                    })
                    .reduce(join_ir_types)
                    .unwrap_or(ir::Type::Unknown);
                ir::Type::list(item_ty)
            }
            Expr::Spread { value, .. } => self.infer_expr_type_with_overrides(value, overrides),
            Expr::TupleLiteral { items, .. } => ir::Type::Tuple(
                items
                    .iter()
                    .map(|item| self.infer_expr_type_with_overrides(item, overrides))
                    .collect(),
            ),
            Expr::ContextualNew { args, style, .. } if *style == core::CallStyle::Brace => args
                .first()
                .map(|arg| self.infer_expr_type_with_overrides(&arg.value, overrides))
                .unwrap_or(ir::Type::Unknown),
            Expr::ContextualNew { .. } => ir::Type::Unknown,
            Expr::RecordLiteral { fields, values, .. } => {
                if fields.is_empty() && !values.is_empty() {
                    return ir::Type::Tuple(
                        values
                            .iter()
                            .map(|value| self.infer_expr_type_with_overrides(value, overrides))
                            .collect(),
                    );
                }
                let explicit_names = fields
                    .iter()
                    .filter_map(|field| field.name.as_deref())
                    .collect::<HashSet<_>>();
                let mut out = Vec::new();
                for field in fields {
                    if let Some(name) = &field.name {
                        upsert_ir_record_field(
                            &mut out,
                            ir::NamedType {
                                name: name.clone(),
                                ty: self.infer_expr_type_with_overrides(&field.value, overrides),
                            },
                        );
                    } else if let Some(spread_fields) = self.infer_record_spread_fields(
                        &self.infer_expr_type_with_overrides(&field.value, overrides),
                    ) {
                        for spread_field in spread_fields {
                            if !explicit_names.contains(spread_field.name.as_str()) {
                                upsert_ir_record_field(&mut out, spread_field);
                            } else if !out.iter().any(|field| field.name == spread_field.name) {
                                out.push(spread_field);
                            }
                        }
                    }
                }
                ir::Type::Record(out)
            }
            Expr::Call {
                callee,
                args,
                style,
                ..
            } => self.infer_call_type(callee, args, *style, overrides),
            Expr::Member { receiver, name, .. }
                if name == "args"
                    && matches!(receiver.as_ref(), Expr::Identifier { name, .. } if name == "OS") =>
            {
                ir::Type::list(ir::Type::Str)
            }
            Expr::Member { receiver, name, .. } => {
                if name == "runtimeType" {
                    let receiver_ty = self.infer_expr_type_with_overrides(receiver, overrides);
                    return ir_value_runtime_type(receiver_ty);
                }
                if name == "referenceId" {
                    return ir::Type::named("ReferenceId");
                }
                let receiver_ty = self.infer_expr_type_with_overrides(receiver, overrides);
                self.infer_member_type(&receiver_ty, name)
                    .unwrap_or(ir::Type::Unknown)
            }
            Expr::Index {
                receiver, index, ..
            } => {
                let receiver_ty = self.infer_expr_type_with_overrides(receiver, overrides);
                match (&receiver_ty, static_tuple_index(index)) {
                    (ir::Type::Tuple(items), Some(index)) => {
                        items.get(index).cloned().unwrap_or(ir::Type::Unknown)
                    }
                    _ => index_result_ir_type(&receiver_ty),
                }
            }
            Expr::RecordUpdate { receiver, .. } => {
                self.infer_expr_type_with_overrides(receiver, overrides)
            }
            Expr::ExtractOr {
                value, fallback, ..
            } => {
                let source_ty = self.infer_expr_type_with_overrides(value, overrides);
                let inner = unwrap_lifted_ir_type(&source_ty)
                    .map(|(_, inner)| inner)
                    .unwrap_or(ir::Type::Unknown);
                if inner == ir::Type::Unknown {
                    self.infer_expr_type_with_overrides(fallback, overrides)
                } else {
                    inner
                }
            }
            Expr::Try { value, .. } => {
                let source_ty = self.infer_expr_type_with_overrides(value, overrides);
                unwrap_lifted_ir_type(&source_ty)
                    .map(|(_, inner)| inner)
                    .unwrap_or(ir::Type::Unknown)
            }
            Expr::Unary {
                op: ast::UnaryOp::OptionWrap,
                expr,
                ..
            } => ir::Type::option(self.infer_expr_type_with_overrides(expr, overrides)),
            Expr::Unary {
                op: ast::UnaryOp::UnsafeExtract,
                expr,
                ..
            } => {
                let source_ty = self.infer_expr_type_with_overrides(expr, overrides);
                unwrap_lifted_ir_type(&source_ty)
                    .map(|(_, inner)| inner)
                    .unwrap_or(ir::Type::Unknown)
            }
            Expr::TypeOf { ty, .. } => ir_exact_runtime_type(self.lower_type_ref(ty)),
            Expr::Binary {
                left, op, right, ..
            } => {
                let left = self.infer_expr_type_with_overrides(left, overrides);
                let right = self.infer_expr_type_with_overrides(right, overrides);
                match op {
                    AstBinaryOp::Or
                    | AstBinaryOp::And
                    | AstBinaryOp::Less
                    | AstBinaryOp::LessEq
                    | AstBinaryOp::Greater
                    | AstBinaryOp::GreaterEq
                    | AstBinaryOp::Eq
                    | AstBinaryOp::NotEq
                    | AstBinaryOp::StrictEq
                    | AstBinaryOp::StrictNotEq => ir::Type::Bool,
                    AstBinaryOp::Add
                        if matches!(&left, ir::Type::Str) || matches!(&right, ir::Type::Str) =>
                    {
                        ir::Type::Str
                    }
                    AstBinaryOp::Add
                    | AstBinaryOp::Sub
                    | AstBinaryOp::Mul
                    | AstBinaryOp::Div
                    | AstBinaryOp::Mod => join_ir_types(left, right),
                    AstBinaryOp::Colon => ir::Type::Unknown,
                }
            }
            Expr::Is { .. } => ir::Type::Bool,
            Expr::If {
                then_block,
                else_branch,
                ..
            } => {
                let then_ty = self.infer_block_type_with_overrides(then_block, overrides);
                let else_ty = match else_branch.as_ref() {
                    core::ElseExprBranch::If(expr) => {
                        self.infer_expr_type_with_overrides(expr, overrides)
                    }
                    core::ElseExprBranch::Block(block) => {
                        self.infer_block_type_with_overrides(block, overrides)
                    }
                };
                join_ir_types(then_ty, else_ty)
            }
            Expr::Block { body, .. } => self.infer_block_type_with_overrides(body, overrides),
            Expr::Return { .. } | Expr::Break { .. } | Expr::Continue { .. } => ir::Type::Never,
            Expr::AnonymousObject { span, .. } => {
                ir::Type::named(crate::source::anonymous_object_type_name(*span))
            }
            Expr::Match { .. }
            | Expr::ForYield { .. }
            | Expr::Unary { .. }
            | Expr::Lambda { .. }
            | Expr::Placeholder { .. } => ir::Type::Unknown,
        }
    }

    fn infer_block_type_with_overrides(
        &self,
        block: &Block,
        overrides: &[(String, ir::Type)],
    ) -> ir::Type {
        block
            .statements
            .last()
            .and_then(|statement| match statement {
                Stmt::Expr(expr) => {
                    Some(self.infer_expr_type_with_overrides(&expr.expr, overrides))
                }
                _ => None,
            })
            .unwrap_or(ir::Type::Unknown)
    }

    fn infer_call_type(
        &self,
        callee: &Expr,
        args: &[core::CallArg],
        style: core::CallStyle,
        overrides: &[(String, ir::Type)],
    ) -> ir::Type {
        if matches!(callee, Expr::Identifier { name, .. } if name == "Any") {
            return ir::Type::named("Any");
        }
        let normalized_args = self.normalize_trailing_brace_call_args(callee, args, style);
        if let Some(ty) = self.infer_builtin_case_call_type(callee, &normalized_args, overrides) {
            return ty;
        }
        if let Some(ty) = self.infer_constructor_call_type(callee, &normalized_args, overrides) {
            return ty;
        }
        if let Some(ty) = self.infer_member_call_type(callee, &normalized_args, overrides) {
            return ty;
        }
        if let Some(ty) = self.infer_named_runtime_call_type(callee) {
            return ty;
        }
        match self.infer_expr_type_with_overrides(callee, overrides) {
            ir::Type::Function { ret, .. } => *ret,
            ir::Type::Named { name, args } if declared_type_exists(self.program, &name) => {
                ir::Type::Named { name, args }
            }
            _ => ir::Type::Unknown,
        }
    }

    fn spread_only_literal_is_map(&self, items: &[Expr], expected: Option<&ir::Type>) -> bool {
        if items.is_empty() || !items.iter().all(|item| matches!(item, Expr::Spread { .. })) {
            return false;
        }

        let mut has_map = false;
        let mut has_known_non_map = false;
        for item in items {
            let Expr::Spread { value, .. } = item else {
                continue;
            };
            match self.infer_expr_type(value) {
                ir::Type::Named { name, args } if name == "Map" && args.len() == 2 => {
                    has_map = true;
                }
                ir::Type::Unknown => {}
                _ => has_known_non_map = true,
            }
        }
        if has_map {
            return !has_known_non_map;
        }
        !has_known_non_map
            && matches!(
                expected,
                Some(ir::Type::Named { name, args }) if name == "Map" && args.len() == 2
            )
    }

    fn infer_spread_map_type(&self, items: &[Expr]) -> ir::Type {
        let mut key = ir::Type::Unknown;
        let mut value = ir::Type::Unknown;
        for item in items {
            let Expr::Spread { value: spread, .. } = item else {
                continue;
            };
            if let ir::Type::Named { name, args } = self.infer_expr_type(spread) {
                if name == "Map" && args.len() == 2 {
                    key = join_ir_types(key, args[0].clone());
                    value = join_ir_types(value, args[1].clone());
                }
            }
        }
        ir::Type::Named {
            name: "Map".to_string(),
            args: vec![key, value],
        }
    }

    fn infer_expr_type_against_with_overrides(
        &self,
        expr: &Expr,
        expected: &ir::Type,
        overrides: &[(String, ir::Type)],
    ) -> ir::Type {
        let actual = match expr {
            Expr::Lambda { params, body, .. } => {
                let expected_params = match expected {
                    ir::Type::Function { params, .. } => params.as_slice(),
                    _ => &[],
                };
                let expected_ret = match expected {
                    ir::Type::Function { ret, .. } => ret.as_ref().clone(),
                    _ => ir::Type::Unknown,
                };
                let param_types = params
                    .iter()
                    .enumerate()
                    .map(|(index, param)| {
                        lower_lambda_param_type(
                            param,
                            expected_params.get(index),
                            self.type_aliases,
                        )
                    })
                    .collect::<Vec<_>>();
                let mut body_overrides = overrides.to_vec();
                for (index, param) in params.iter().enumerate() {
                    if param.name != "_" && param.destructure.is_none() {
                        body_overrides.push((
                            param.name.clone(),
                            param_types.get(index).cloned().unwrap_or(ir::Type::Unknown),
                        ));
                    }
                }
                let ret = self.infer_lambda_body_type_against(body, &expected_ret, &body_overrides);
                ir::Type::Function {
                    params: param_types,
                    ret: Box::new(ret),
                }
            }
            _ => self.infer_expr_type_with_overrides(expr, overrides),
        };
        contextualize_inferred_ir_type(actual, expected)
    }

    fn infer_lambda_body_type_against(
        &self,
        body: &Expr,
        expected: &ir::Type,
        overrides: &[(String, ir::Type)],
    ) -> ir::Type {
        let actual = match body {
            Expr::Block { body, .. } => {
                body.statements
                    .last()
                    .and_then(|statement| match statement {
                        Stmt::Expr(expr) => Some(self.infer_expr_type_against_with_overrides(
                            &expr.expr, expected, overrides,
                        )),
                        _ => None,
                    })
                    .unwrap_or(ir::Type::Unknown)
            }
            _ => self.infer_expr_type_against_with_overrides(body, expected, overrides),
        };
        contextualize_inferred_ir_type(actual, expected)
    }

    fn infer_function_call_subst(
        &self,
        function_id: ir::FunctionId,
        args: &[core::CallArg],
        subst: &mut HashMap<String, ir::Type>,
    ) {
        let Some(function) = self.program.function(function_id) else {
            return;
        };
        for (arg, param_index) in args.iter().zip(source_param_indices(function)) {
            let Some(param) = function.params.get(param_index) else {
                continue;
            };
            let Some(local) = function.locals.get(param.0) else {
                continue;
            };
            let expected = substitute_ir_type(&local.ty, subst);
            let actual = self.infer_expr_type_against_with_overrides(&arg.value, &expected, &[]);
            infer_ir_type_subst(&expected, &actual, subst);
        }
    }

    fn infer_named_runtime_call_type(&self, callee: &Expr) -> Option<ir::Type> {
        let path = expr_path(callee)?;
        match path.as_slice() {
            [owner, method] if owner == "Int" && method == "parse" => {
                Some(ir::Type::option(ir::Type::Int))
            }
            [owner, method] if owner == "Float" && method == "parse" => {
                Some(ir::Type::option(ir::Type::Float))
            }
            [owner, method] if owner == "File" && method == "readBytes" => Some(ir::Type::Named {
                name: "Result".to_string(),
                args: vec![ir::Type::list(ir::Type::Int), ir::Type::named("FileError")],
            }),
            [owner, method] if owner == "File" && method == "readText" => Some(ir::Type::Named {
                name: "Result".to_string(),
                args: vec![ir::Type::Str, ir::Type::named("FileError")],
            }),
            [owner, method] if owner == "File" && method == "open" => Some(ir::Type::Named {
                name: "Result".to_string(),
                args: vec![ir::Type::named("FileStream"), ir::Type::named("FileError")],
            }),
            [owner, method] if owner == "File" && method == "openText" => Some(ir::Type::Named {
                name: "Result".to_string(),
                args: vec![
                    ir::Type::named("TextFileReader"),
                    ir::Type::named("FileError"),
                ],
            }),
            [owner] if owner == "Map" => Some(ir::Type::Named {
                name: "Map".to_string(),
                args: vec![ir::Type::Unknown, ir::Type::Unknown],
            }),
            _ => None,
        }
    }

    fn infer_builtin_case_call_type(
        &self,
        callee: &Expr,
        args: &[core::CallArg],
        overrides: &[(String, ir::Type)],
    ) -> Option<ir::Type> {
        let path = expr_path(callee)?;
        let name = path.last()?.as_str();
        let first_arg = args
            .first()
            .map(|arg| self.infer_expr_type_with_overrides(&arg.value, overrides))
            .unwrap_or(ir::Type::Unknown);
        match name {
            "Some" => Some(ir::Type::option(first_arg)),
            "Ok" => Some(ir::Type::Named {
                name: "Result".to_string(),
                args: vec![first_arg, ir::Type::Unknown],
            }),
            "Err" => Some(ir::Type::Named {
                name: "Result".to_string(),
                args: vec![ir::Type::Unknown, first_arg],
            }),
            "Left" => Some(ir::Type::Named {
                name: "Either".to_string(),
                args: vec![first_arg, ir::Type::Unknown],
            }),
            "Right" => Some(ir::Type::Named {
                name: "Either".to_string(),
                args: vec![ir::Type::Unknown, first_arg],
            }),
            _ => None,
        }
    }

    fn infer_constructor_call_type(
        &self,
        callee: &Expr,
        args: &[core::CallArg],
        overrides: &[(String, ir::Type)],
    ) -> Option<ir::Type> {
        let (callee, explicit_type_args) = self.split_generic_call_callee(callee);
        let path = self.canonical_enum_case_path(&expr_path(callee)?);
        let declared_name = self.canonical_declared_type_path(&path);
        if declared_name.is_some()
            || (path.len() == 1 && runtime_collection_constructor_name(&path[0]))
        {
            let name = declared_name.as_ref().unwrap_or(&path[0]);
            let type_params = self
                .program
                .types
                .iter()
                .find(|ty| ty.name == *name)
                .map(|ty| ty.type_params.clone())
                .unwrap_or_else(|| {
                    if name == "Map" {
                        vec!["K".to_string(), "V".to_string()]
                    } else {
                        vec!["T".to_string()]
                    }
                });
            let mut subst = type_params
                .iter()
                .cloned()
                .zip(explicit_type_args)
                .collect::<HashMap<_, _>>();

            if subst.is_empty()
                && matches!(name.as_str(), "Vector" | "LinkedList" | "Array" | "Set")
            {
                let mut item = ir::Type::Unknown;
                for arg in args {
                    item = join_ir_types(
                        item,
                        self.infer_expr_type_with_overrides(&arg.value, overrides),
                    );
                }
                if !matches!(item, ir::Type::Unknown)
                    && let Some(param) = type_params.first()
                {
                    subst.insert(param.clone(), item);
                }
            } else if subst.is_empty() && name == "Map" {
                let mut key = ir::Type::Unknown;
                let mut value = ir::Type::Unknown;
                for arg in args {
                    if let ir::Type::Tuple(items) =
                        self.infer_expr_type_with_overrides(&arg.value, overrides)
                        && items.len() == 2
                    {
                        key = join_ir_types(key, items[0].clone());
                        value = join_ir_types(value, items[1].clone());
                    }
                }
                if let Some(param) = type_params.first()
                    && !matches!(key, ir::Type::Unknown)
                {
                    subst.insert(param.clone(), key);
                }
                if let Some(param) = type_params.get(1)
                    && !matches!(value, ir::Type::Unknown)
                {
                    subst.insert(param.clone(), value);
                }
            } else if subst.is_empty()
                && let Some(ty) = self.program.types.iter().find(|ty| ty.name == *name)
            {
                if let Some(constructor) = ty.methods.iter().find_map(|method_id| {
                    let function = self.program.function(*method_id)?;
                    (function.name == "new" && method_call_arity_score(function, args).is_some())
                        .then_some(*method_id)
                }) {
                    self.infer_function_call_subst(constructor, args, &mut subst);
                } else {
                    for (field, arg) in ty.fields.iter().zip(args) {
                        let actual = self.infer_expr_type_with_overrides(&arg.value, overrides);
                        infer_ir_type_subst(&field.ty, &actual, &mut subst);
                    }
                }
            }

            return Some(ir::Type::Named {
                name: name.clone(),
                args: type_params
                    .iter()
                    .map(|param| subst.get(param).cloned().unwrap_or(ir::Type::Unknown))
                    .collect(),
            });
        }
        self.lookup_enum_case_type_by_path(&path)
    }

    fn lookup_override_type(
        &self,
        name: &str,
        overrides: &[(String, ir::Type)],
    ) -> Option<ir::Type> {
        overrides
            .iter()
            .rev()
            .find(|(candidate, _)| candidate == name)
            .map(|(_, ty)| ty.clone())
    }

    fn infer_record_spread_fields(&self, ty: &ir::Type) -> Option<Vec<ir::NamedType>> {
        match ty {
            ir::Type::Record(fields) => Some(fields.clone()),
            ir::Type::Named { name, args } => {
                let ty = self.program.types.iter().find(|ty| {
                    ty.name == *name
                        && matches!(ty.kind, ast::TypeKind::Class | ast::TypeKind::Record)
                })?;
                if ty.fields.is_empty() {
                    return None;
                }
                let subst = ir_type_subst(ty, args);
                Some(
                    ty.fields
                        .iter()
                        .filter(|field| field.visibility != ast::Visibility::Private)
                        .map(|field| ir::NamedType {
                            name: field.name.clone(),
                            ty: substitute_ir_type(&field.ty, &subst),
                        })
                        .collect(),
                )
            }
            _ => None,
        }
    }

    fn lookup_scoped_type(&self, name: &str) -> Option<ir::Type> {
        for scope in self.scopes.iter().rev() {
            if let Some(local) = scope.get(name).copied() {
                if let Some(value_ty) = self.lazy_values.get(name) {
                    return Some(value_ty.clone());
                }
                return self
                    .function()
                    .locals
                    .get(local.0)
                    .map(|local| local.ty.clone());
            }
        }
        self.capture_sources.get(name).map(|source| {
            source
                .lazy_value_ty
                .clone()
                .unwrap_or_else(|| source.ty.clone())
        })
    }

    fn type_narrowing_for_condition(
        &self,
        condition: &Expr,
        condition_is_true: bool,
    ) -> Option<IrTypeNarrowing> {
        match condition {
            Expr::Unary {
                op: ast::UnaryOp::Not,
                expr,
                ..
            } => self.type_narrowing_for_condition(expr, !condition_is_true),
            Expr::Is { left, target, span } if condition_is_true => {
                let Expr::Identifier { name, .. } = left.as_ref() else {
                    return None;
                };
                if self.lazy_values.contains_key(name)
                    || lower_runtime_type_ref_has_arguments(target)
                {
                    return None;
                }
                let local = self.lookup_scoped_local(name)?;
                let local_info = self.function().locals.get(local.0)?;
                if local_info.mutable {
                    return None;
                }
                let ty = self.lower_type_ref(target);
                if matches!(ty, ir::Type::Unknown | ir::Type::TypeParam(_)) {
                    return None;
                }
                Some(IrTypeNarrowing {
                    name: name.clone(),
                    ty,
                    span: *span,
                })
            }
            _ => None,
        }
    }

    fn apply_type_narrowing(&mut self, narrowing: Option<&IrTypeNarrowing>) {
        let Some(narrowing) = narrowing else {
            return;
        };
        let Some(source) = self.lookup_scoped_or_captured_value(&narrowing.name) else {
            return;
        };
        let local = self.add_temp(narrowing.ty.clone());
        self.push_statement(ir::Statement {
            span: Some(narrowing.span),
            kind: ir::StatementKind::Assign {
                target: ir::Place::Local(local),
                value: ir::RValue::Cast {
                    operand: source,
                    ty: narrowing.ty.clone(),
                },
            },
        });
        self.bind_existing(&narrowing.name, local);
    }

    fn lookup_scoped_local(&self, name: &str) -> Option<ir::LocalId> {
        self.scopes
            .iter()
            .rev()
            .find_map(|scope| scope.get(name).copied())
    }

    fn operand_type(&self, operand: &ir::Operand) -> Option<ir::Type> {
        match operand {
            ir::Operand::Copy(place) | ir::Operand::Move(place) => self.place_type(place),
            ir::Operand::Const(constant) => Some(match constant {
                ir::Constant::Unit => ir::Type::Unit,
                ir::Constant::OptionNone => ir::Type::option(ir::Type::Unknown),
                ir::Constant::Bool(_) => ir::Type::Bool,
                ir::Constant::Int(_) => ir::Type::Int,
                ir::Constant::Float(_) => ir::Type::Float,
                ir::Constant::String(_) => ir::Type::Str,
                ir::Constant::List(values) => ir::Type::list(
                    values
                        .first()
                        .and_then(|value| self.operand_type(&ir::Operand::Const(value.clone())))
                        .unwrap_or(ir::Type::Unknown),
                ),
            }),
        }
    }

    fn place_type(&self, place: &ir::Place) -> Option<ir::Type> {
        match place {
            ir::Place::Local(local) => self
                .function()
                .locals
                .get(local.0)
                .map(|local| local.ty.clone()),
            ir::Place::Global(global) => self
                .program
                .globals
                .get(global.0)
                .map(|global| global.ty.clone()),
            ir::Place::Field { .. } | ir::Place::Index { .. } => None,
        }
    }

    fn lookup_global_type(&self, name: &str) -> Option<ir::Type> {
        let global = self.globals.get(name)?;
        self.program
            .globals
            .get(global.0)
            .map(|global| global.ty.clone())
    }

    fn lookup_function_type(&self, name: &str) -> Option<ir::Type> {
        let function = self.functions.get(name).copied()?;
        self.function_type(function)
    }

    fn lookup_declared_type_value(&self, name: &str) -> Option<ir::Type> {
        declared_type_exists(self.program, name).then(|| ir::Type::Named {
            name: name.to_string(),
            args: Vec::new(),
        })
    }

    fn lookup_bare_enum_case_type(&self, name: &str) -> Option<ir::Type> {
        if name == "None" {
            return Some(ir::Type::option(ir::Type::Unknown));
        }
        self.lookup_enum_case_type_by_path(&[name.to_string()])
    }

    fn lower_type_test_target(&self, target: &TypeRef) -> ir::Type {
        if let TypeRef::Named { name, args, .. } = target
            && args.is_empty()
        {
            let owner = match name.as_str() {
                "Some" | "None" => Some("Option"),
                "Ok" | "Err" => Some("Result"),
                "Left" | "Right" => Some("Either"),
                _ => unique_bare_enum_case_owner(self.program, name),
            };
            if let Some(owner) = owner {
                return ir::Type::Named {
                    name: format!("{owner}::{name}"),
                    args: Vec::new(),
                };
            }
        }
        self.lower_type_ref(target)
    }

    fn lookup_enum_case_type_by_path(&self, path: &[String]) -> Option<ir::Type> {
        let path = self.canonical_enum_case_path(path);
        let matches = self
            .program
            .types
            .iter()
            .filter(|ty| ty.kind == ast::TypeKind::Enum)
            .filter(|ty| {
                if path.len() == 2 {
                    ty.name == path[0] && ty.enum_cases.iter().any(|case| case.name == path[1])
                } else {
                    path.len() == 1 && ty.enum_cases.iter().any(|case| case.name == path[0])
                }
            })
            .collect::<Vec<_>>();
        let ty = (matches.len() == 1).then(|| matches[0])?;
        Some(ir::Type::Named {
            name: ty.name.clone(),
            args: ty
                .type_params
                .iter()
                .map(|param| ir::Type::TypeParam(param.clone()))
                .collect(),
        })
    }

    fn lookup_implicit_field_type(&self, name: &str) -> Option<ir::Type> {
        let this_ty = if let Some(this_local) = self.this_local {
            self.function().locals.get(this_local.0)?.ty.clone()
        } else {
            self.capture_sources.get("this")?.ty.clone()
        };
        let ir::Type::Named {
            name: type_name,
            args,
        } = this_ty
        else {
            return None;
        };
        let ty = self.program.types.iter().find(|ty| ty.name == type_name)?;
        let subst = ir_type_subst(ty, &args);
        ty.fields
            .iter()
            .find(|field| field.name == name)
            .map(|field| substitute_ir_type(&field.ty, &subst))
    }

    fn lookup_implicit_getter_type(&self, name: &str) -> Option<ir::Type> {
        let this_ty = if let Some(this_local) = self.this_local {
            self.function().locals.get(this_local.0)?.ty.clone()
        } else {
            self.capture_sources.get("this")?.ty.clone()
        };
        let (function, subst) = self.getter_function_for_type(&this_ty, name)?;
        let function = self.program.function(function)?;
        Some(substitute_ir_type(&function.return_ty, &subst))
    }

    fn getter_function_for_type(
        &self,
        receiver: &ir::Type,
        name: &str,
    ) -> Option<(ir::FunctionId, HashMap<String, ir::Type>)> {
        match receiver {
            ir::Type::Named {
                name: type_name,
                args,
            } => {
                if let Some((owner_name, _)) = enum_case_view_parts(type_name) {
                    return self.getter_function_for_type(
                        &ir::Type::Named {
                            name: owner_name.to_string(),
                            args: args.clone(),
                        },
                        name,
                    );
                }
                let ty = self.program.types.iter().find(|ty| ty.name == *type_name)?;
                if ty.fields.iter().any(|field| field.name == name) {
                    return None;
                }
                let function = self.find_method_function(ty, name)?;
                self.program
                    .function(function)
                    .is_some_and(|function| function.getter)
                    .then(|| (function, ir_type_subst(ty, args)))
            }
            ir::Type::TypeParam(param) => self
                .generic_bounds_for_type_param(param)
                .into_iter()
                .find_map(|bound| self.getter_function_for_type(&bound, name)),
            _ => None,
        }
    }

    fn is_getter_member_for_type(&self, receiver: &ir::Type, name: &str) -> bool {
        self.getter_function_for_type(receiver, name).is_some()
            || builtin_getter_type(receiver, name).is_some()
            || (is_known_getter_name(name) && !self.type_has_data_field(receiver, name))
    }

    fn type_has_data_field(&self, receiver: &ir::Type, name: &str) -> bool {
        match receiver {
            ir::Type::Named {
                name: type_name, ..
            } => {
                if let Some((owner_name, case_name)) = enum_case_view_parts(type_name) {
                    return self
                        .program
                        .types
                        .iter()
                        .find(|ty| ty.name == owner_name)
                        .is_some_and(|ty| {
                            ty.enum_cases
                                .iter()
                                .find(|case| case.name == case_name)
                                .is_some_and(|case| {
                                    case.fields.iter().any(|field| field.name == name)
                                })
                                || ty.fields.iter().any(|field| field.name == name)
                        });
                }
                self.program
                    .types
                    .iter()
                    .find(|ty| ty.name == *type_name)
                    .is_some_and(|ty| ty.fields.iter().any(|field| field.name == name))
            }
            ir::Type::Record(fields) => fields.iter().any(|field| field.name == name),
            _ => false,
        }
    }

    fn infer_member_type(&self, receiver: &ir::Type, name: &str) -> Option<ir::Type> {
        match receiver {
            ir::Type::Named {
                name: type_name,
                args,
            } => {
                if let Some((owner_name, case_name)) = enum_case_view_parts(type_name) {
                    if let Some(field_ty) =
                        builtin_enum_case_view_field_type(owner_name, case_name, name, args)
                    {
                        return Some(field_ty);
                    }
                    let owner = self
                        .program
                        .types
                        .iter()
                        .find(|ty| ty.kind == ast::TypeKind::Enum && ty.name == owner_name)?;
                    let subst = ir_type_subst(owner, args);
                    if let Some(field) = owner
                        .enum_cases
                        .iter()
                        .find(|case| case.name == case_name)
                        .and_then(|case| case.fields.iter().find(|field| field.name == name))
                    {
                        return Some(substitute_ir_type(&field.ty, &subst));
                    }
                    return self.infer_member_type(
                        &ir::Type::Named {
                            name: owner_name.to_string(),
                            args: args.clone(),
                        },
                        name,
                    );
                }
                let Some(ty) = self.program.types.iter().find(|ty| ty.name == *type_name) else {
                    return self
                        .extension_member_type(type_name, args, name)
                        .or_else(|| builtin_member_type(receiver, name));
                };
                let subst = ir_type_subst(ty, args);
                if let Some(field) = ty.fields.iter().find(|field| field.name == name) {
                    return Some(substitute_ir_type(&field.ty, &subst));
                }
                if let Some(function) = self.find_method_function(ty, name) {
                    let method_ty = self.function_type_with_subst(function, &subst)?;
                    if self
                        .program
                        .function(function)
                        .is_some_and(|function| function.getter)
                    {
                        let ir::Type::Function { ret, .. } = method_ty else {
                            return None;
                        };
                        return Some(*ret);
                    }
                    if function_type_returns_unknown(&method_ty) {
                        if let Some(fallback) = builtin_member_type(receiver, name) {
                            return Some(fallback);
                        }
                    }
                    return Some(method_ty);
                }
                builtin_member_type(receiver, name)
            }
            ir::Type::Bool | ir::Type::Int | ir::Type::Float | ir::Type::Str => {
                let type_name = builtin_extension_receiver_name(receiver)?;
                self.extension_member_type(type_name, &[], name)
                    .or_else(|| builtin_member_type(receiver, name))
            }
            ir::Type::Record(fields) => fields
                .iter()
                .find(|field| field.name == name)
                .map(|field| field.ty.clone()),
            ir::Type::TypeParam(param) => self
                .generic_bounds_for_type_param(param)
                .into_iter()
                .find_map(|bound| self.infer_member_type(&bound, name)),
            ir::Type::Unknown => Some(ir::Type::Unknown),
            _ => builtin_member_type(receiver, name),
        }
    }

    fn extension_member_type(
        &self,
        type_name: &str,
        args: &[ir::Type],
        name: &str,
    ) -> Option<ir::Type> {
        let ty = self.program.types.iter().find(|ty| {
            ty.name == type_name
                && matches!(
                    ty.kind,
                    ast::TypeKind::Class
                        | ast::TypeKind::Record
                        | ast::TypeKind::Enum
                        | ast::TypeKind::Interface
                )
        })?;
        let subst = ir_type_subst(ty, args);
        let function = self.find_method_function(ty, name)?;
        let method_ty = self.function_type_with_subst(function, &subst)?;
        if self
            .program
            .function(function)
            .is_some_and(|function| function.getter)
        {
            let ir::Type::Function { ret, .. } = method_ty else {
                return None;
            };
            Some(*ret)
        } else {
            Some(method_ty)
        }
    }

    fn is_callable_reference_expr(&self, expr: &Expr) -> bool {
        match expr {
            Expr::Identifier { name, .. } => self.is_top_level_function_reference(name),
            Expr::Member { receiver, name, .. } => self.is_bound_method_reference(receiver, name),
            _ => false,
        }
    }

    fn callable_reference_type(&self, expr: &Expr) -> Option<ir::Type> {
        if !self.is_callable_reference_expr(expr) {
            return None;
        }
        match expr {
            Expr::Identifier { name, .. } => self.lookup_function_type(name),
            Expr::Member { receiver, name, .. } => {
                self.callable_reference_member_type(receiver, name)
            }
            _ => None,
        }
    }

    fn is_top_level_function_reference(&self, name: &str) -> bool {
        if self.lookup_scoped_type(name).is_some()
            || self.lookup_implicit_field_type(name).is_some()
            || self.lookup_global_type(name).is_some()
            || self.lookup_bare_enum_case_type(name).is_some()
            || self.lookup_declared_type_value(name).is_some()
        {
            return false;
        }
        self.functions.contains_key(name)
    }

    fn is_bound_method_reference(&self, receiver: &Expr, name: &str) -> bool {
        if self.is_getter_member_for_type(&self.infer_expr_type(receiver), name) {
            return false;
        }
        self.callable_reference_member_type(receiver, name)
            .is_some()
    }

    fn callable_reference_member_type(&self, receiver: &Expr, name: &str) -> Option<ir::Type> {
        let (ty, args) = self.callable_reference_receiver_type_def(receiver)?;
        if ty.fields.iter().any(|field| field.name == name) {
            return None;
        }
        let subst = ir_type_subst(ty, &args);
        let function = self.find_method_function(ty, name)?;
        if self
            .program
            .function(function)
            .is_some_and(|function| function.getter)
        {
            return None;
        }
        self.function_type_with_subst(function, &subst)
    }

    fn callable_reference_receiver_type_def(
        &self,
        receiver: &Expr,
    ) -> Option<(&ir::TypeDef, Vec<ir::Type>)> {
        if let Expr::Identifier { name, .. } = receiver {
            let value_in_scope = self.lookup_scoped_type(name).is_some()
                || self.lookup_implicit_field_type(name).is_some()
                || self.lookup_global_type(name).is_some();
            if !value_in_scope && single_type_exists(self.program, name) {
                let ty = self
                    .program
                    .types
                    .iter()
                    .find(|ty| ty.name == *name && ty.kind == ast::TypeKind::Object)?;
                return Some((ty, Vec::new()));
            }
            if self.lookup_scoped_type(name).is_none()
                && self.lookup_implicit_field_type(name).is_none()
                && self.lookup_global_type(name).is_none()
                && declared_type_exists(self.program, name)
            {
                return None;
            }
        }
        let ir::Type::Named { name, args } = self.infer_expr_type(receiver) else {
            return None;
        };
        let ty = self.program.types.iter().find(|ty| ty.name == name)?;
        Some((ty, args))
    }

    fn infer_member_call_type(
        &self,
        callee: &Expr,
        args: &[core::CallArg],
        overrides: &[(String, ir::Type)],
    ) -> Option<ir::Type> {
        let Expr::Member { receiver, name, .. } = callee else {
            return None;
        };
        if matches!(receiver.as_ref(), Expr::Identifier { name, .. } if name == "Math")
            && matches!(name.as_str(), "min" | "max")
            && args.len() == 2
        {
            let left = self.infer_expr_type_with_overrides(&args[0].value, overrides);
            let right = self.infer_expr_type_with_overrides(&args[1].value, overrides);
            return match (&left, &right) {
                (ir::Type::Int, ir::Type::Int) => Some(ir::Type::Int),
                (ir::Type::Float, ir::Type::Float) => Some(ir::Type::Float),
                _ => Some(ir::Type::Unknown),
            };
        }
        let receiver_ty = self.infer_expr_type_with_overrides(receiver, overrides);
        let ir::Type::Named {
            name: type_name,
            args: type_args,
        } = &receiver_ty
        else {
            if let ir::Type::TypeParam(param) = &receiver_ty {
                for bound in self.generic_bounds_for_type_param(param) {
                    let ir::Type::Named {
                        name: bound_name,
                        args: bound_args,
                    } = bound
                    else {
                        continue;
                    };
                    let Some(bound_ty) = self.program.types.iter().find(|ty| ty.name == bound_name)
                    else {
                        continue;
                    };
                    let subst = ir_type_subst(bound_ty, &bound_args);
                    if let Some(ret) =
                        self.find_method_call_return_type(bound_ty, name, &subst, args)
                    {
                        return Some(ret);
                    }
                }
            }
            return builtin_member_type(&receiver_ty, name).and_then(|ty| match ty {
                ir::Type::Function { ret, .. } => Some(*ret),
                _ => None,
            });
        };
        if matches!(
            type_name.as_str(),
            "Vector" | "LinkedList" | "Array" | "Set"
        ) {
            match (name.as_str(), args) {
                ("filter" | "sort" | "take", [_]) => return Some(receiver_ty),
                ("fold" | "reduce", [initial, _]) => {
                    return Some(self.infer_expr_type_with_overrides(&initial.value, overrides));
                }
                ("map", [mapper]) => {
                    let item = type_args.first().cloned().unwrap_or(ir::Type::Unknown);
                    let expected = ir::Type::Function {
                        params: vec![item],
                        ret: Box::new(ir::Type::Unknown),
                    };
                    let mapped = self.infer_expr_type_against_with_overrides(
                        &mapper.value,
                        &expected,
                        overrides,
                    );
                    let ir::Type::Function { ret, .. } = mapped else {
                        return Some(ir::Type::Unknown);
                    };
                    return Some(ir::Type::Named {
                        name: type_name.clone(),
                        args: vec![*ret],
                    });
                }
                ("flatMap", [mapper]) => {
                    let item = type_args.first().cloned().unwrap_or(ir::Type::Unknown);
                    let expected = ir::Type::Function {
                        params: vec![item],
                        ret: Box::new(ir::Type::Unknown),
                    };
                    let mapped = self.infer_expr_type_against_with_overrides(
                        &mapper.value,
                        &expected,
                        overrides,
                    );
                    let ir::Type::Function { ret, .. } = mapped else {
                        return Some(ir::Type::Unknown);
                    };
                    if type_name == "Vector" {
                        let mapped_item =
                            known_iterable_ir_item_type(&ret).unwrap_or(ir::Type::Unknown);
                        return Some(ir::Type::Named {
                            name: "Vector".to_string(),
                            args: vec![mapped_item],
                        });
                    }
                    return Some(*ret);
                }
                ("flatten", []) if type_name == "Vector" => {
                    let item = type_args.first().cloned().unwrap_or(ir::Type::Unknown);
                    let flattened = known_iterable_ir_item_type(&item).unwrap_or(ir::Type::Unknown);
                    return Some(ir::Type::list(flattened));
                }
                _ => {}
            }
        }
        let Some(ty) = self.program.types.iter().find(|ty| ty.name == *type_name) else {
            return builtin_member_type(&receiver_ty, name).and_then(|ty| match ty {
                ir::Type::Function { ret, .. } => Some(*ret),
                _ => None,
            });
        };
        let subst = ir_type_subst(ty, type_args);
        let return_ty = self.find_method_call_return_type(ty, name, &subst, args)?;
        if matches!(return_ty, ir::Type::Unknown) {
            if let Some(fallback) =
                builtin_member_type(&receiver_ty, name).and_then(|ty| match ty {
                    ir::Type::Function { ret, .. } => Some(*ret),
                    _ => None,
                })
            {
                return Some(fallback);
            }
        }
        Some(return_ty)
    }

    fn generic_bounds_for_type_param(&self, name: &str) -> Vec<ir::Type> {
        let owner_conditions = match self.function().kind {
            ir::FunctionKind::Method { owner } => self
                .program
                .types
                .get(owner.0)
                .map(|ty| ty.generic_conditions.as_slice())
                .unwrap_or(&[]),
            _ => &[],
        };
        self.function()
            .generic_conditions
            .iter()
            .chain(owner_conditions.iter())
            .filter_map(|condition| match condition {
                ir::GenericCondition::Bound {
                    subject: ir::Type::TypeParam(subject),
                    bound,
                } if subject == name => Some(bound.clone()),
                _ => None,
            })
            .collect()
    }

    fn find_method_call_return_type(
        &self,
        ty: &ir::TypeDef,
        name: &str,
        subst: &HashMap<String, ir::Type>,
        args: &[core::CallArg],
    ) -> Option<ir::Type> {
        let mut seen = Vec::new();
        self.find_method_call_return_type_inner(ty, name, subst, args, &mut seen)
    }

    fn find_method_call_return_type_inner(
        &self,
        ty: &ir::TypeDef,
        name: &str,
        subst: &HashMap<String, ir::Type>,
        args: &[core::CallArg],
        seen: &mut Vec<String>,
    ) -> Option<ir::Type> {
        if seen.iter().any(|item| item == &ty.name) {
            return None;
        }
        seen.push(ty.name.clone());
        let mut best: Option<(usize, ir::Type)> = None;
        for method_id in &ty.methods {
            let Some(function) = self.program.function(*method_id) else {
                continue;
            };
            if function.name != name {
                continue;
            }
            let Some(score) = method_call_arity_score(function, args) else {
                continue;
            };
            if best
                .as_ref()
                .map(|(best_score, _)| score > *best_score)
                .unwrap_or(true)
            {
                let mut call_subst = subst.clone();
                self.infer_function_call_subst(*method_id, args, &mut call_subst);
                best = Some((score, substitute_ir_type(&function.return_ty, &call_subst)));
            }
        }
        if let Some((_, return_ty)) = best {
            return Some(return_ty);
        }

        for bound in &ty.with_bounds {
            let ir::Type::Named {
                name: bound_name, ..
            } = bound
            else {
                continue;
            };
            let Some(bound_ty) = self.program.types.iter().find(|ty| ty.name == *bound_name) else {
                continue;
            };
            if let Some(return_ty) =
                self.find_method_call_return_type_inner(bound_ty, name, subst, args, seen)
            {
                return Some(return_ty);
            }
        }
        None
    }

    fn find_method_function(&self, ty: &ir::TypeDef, name: &str) -> Option<ir::FunctionId> {
        let mut seen = Vec::new();
        self.find_method_function_inner(ty, name, &mut seen)
    }

    fn find_method_function_inner(
        &self,
        ty: &ir::TypeDef,
        name: &str,
        seen: &mut Vec<String>,
    ) -> Option<ir::FunctionId> {
        if seen.iter().any(|item| item == &ty.name) {
            return None;
        }
        seen.push(ty.name.clone());
        if let Some(method) = ty.methods.iter().copied().find(|method_id| {
            self.program
                .function(*method_id)
                .is_some_and(|function| function.name == name)
        }) {
            return Some(method);
        }
        for bound in &ty.with_bounds {
            let ir::Type::Named {
                name: bound_name, ..
            } = bound
            else {
                continue;
            };
            let Some(bound_ty) = self.program.types.iter().find(|ty| ty.name == *bound_name) else {
                continue;
            };
            if let Some(method) = self.find_method_function_inner(bound_ty, name, seen) {
                return Some(method);
            }
        }
        None
    }

    fn function_type(&self, function: ir::FunctionId) -> Option<ir::Type> {
        self.function_type_with_subst(function, &HashMap::new())
    }

    fn function_type_with_subst(
        &self,
        function: ir::FunctionId,
        subst: &HashMap<String, ir::Type>,
    ) -> Option<ir::Type> {
        let function = self.program.function(function)?;
        Some(ir::Type::Function {
            params: function
                .params
                .iter()
                .filter_map(|param| function.locals.get(param.0))
                .map(|local| substitute_ir_type(&local.ty, subst))
                .collect(),
            ret: Box::new(substitute_ir_type(&function.return_ty, subst)),
        })
    }

    fn lower_logical_expr(
        &mut self,
        left: &Expr,
        op: AstBinaryOp,
        right: &Expr,
        span: Span,
    ) -> ir::Operand {
        let temp = self.add_temp(ir::Type::Bool);
        let right_block = self.add_block();
        let short_block = self.add_block();
        let join_block = self.add_block();

        let left_value = self.lower_expr(left);
        let (then_block, else_block, short_value) = match op {
            AstBinaryOp::And => (right_block, short_block, false),
            AstBinaryOp::Or => (short_block, right_block, true),
            _ => unreachable!(),
        };
        self.terminate(ir::Terminator {
            span: Some(span),
            kind: ir::TerminatorKind::Branch {
                condition: left_value,
                then_block,
                else_block,
            },
        });

        self.current_block = Some(short_block);
        self.push_statement(ir::Statement {
            span: Some(span),
            kind: ir::StatementKind::Assign {
                target: ir::Place::Local(temp),
                value: ir::RValue::Use(ir::Operand::Const(ir::Constant::Bool(short_value))),
            },
        });
        if self.current_block.is_some() {
            self.terminate(ir::Terminator::goto(join_block));
        }

        self.current_block = Some(right_block);
        let right_value = self.lower_expr(right);
        self.push_statement(ir::Statement {
            span: Some(span),
            kind: ir::StatementKind::Assign {
                target: ir::Place::Local(temp),
                value: ir::RValue::Use(right_value),
            },
        });
        if self.current_block.is_some() {
            self.terminate(ir::Terminator::goto(join_block));
        }

        let join_used = self.block_has_predecessor(join_block);
        self.current_block = if join_used { Some(join_block) } else { None };
        ir::Operand::Copy(Box::new(ir::Place::Local(temp)))
    }

    fn lower_if_expr(
        &mut self,
        condition_clauses: &[core::IfConditionClause],
        then_block: &Block,
        else_branch: &ElseExprBranch,
        span: Span,
        expected: Option<&ir::Type>,
    ) -> ir::Operand {
        let else_narrowing = match condition_clauses {
            [core::IfConditionClause::Expr(condition)] => {
                self.type_narrowing_for_condition(condition, false)
            }
            _ => None,
        };
        let temp = self.add_temp(expected.cloned().unwrap_or(ir::Type::Unknown));
        let then_id = self.add_block();
        let else_id = self.add_block();
        let join_id = self.add_block();

        self.push_scope();
        self.lower_if_condition_clause_chain(condition_clauses, then_id, else_id);

        self.current_block = Some(then_id);
        if let Some(value) = self.lower_block_value_with_expected(then_block, expected) {
            self.push_statement(ir::Statement {
                span: Some(then_block.span),
                kind: ir::StatementKind::Assign {
                    target: ir::Place::Local(temp),
                    value: ir::RValue::Use(value),
                },
            });
        }
        self.pop_scope();
        if self.current_block.is_some() {
            self.terminate(ir::Terminator::goto(join_id));
        }

        self.current_block = Some(else_id);
        self.push_scope();
        self.apply_type_narrowing(else_narrowing.as_ref());
        let else_value = match else_branch {
            ElseExprBranch::If(expr) => Some(self.lower_expr_with_expected(expr, expected)),
            ElseExprBranch::Block(block) => self.lower_block_value_with_expected(block, expected),
        };
        if let Some(value) = else_value {
            self.push_statement(ir::Statement {
                span: Some(span),
                kind: ir::StatementKind::Assign {
                    target: ir::Place::Local(temp),
                    value: ir::RValue::Use(value),
                },
            });
        }
        self.pop_scope();
        if self.current_block.is_some() {
            self.terminate(ir::Terminator::goto(join_id));
        }

        let join_used = self.block_has_predecessor(join_id);
        self.current_block = if join_used { Some(join_id) } else { None };
        ir::Operand::Copy(Box::new(ir::Place::Local(temp)))
    }

    fn lower_block_expr(&mut self, block: &Block) -> ir::Operand {
        self.lower_block_value(block)
            .unwrap_or(ir::Operand::Const(ir::Constant::Unit))
    }

    fn lower_rvalue(&mut self, expr: &Expr) -> Option<ir::RValue> {
        self.lower_rvalue_with_expected(expr, None)
    }

    fn lower_rvalue_with_expected(
        &mut self,
        expr: &Expr,
        expected: Option<&ir::Type>,
    ) -> Option<ir::RValue> {
        if let Some(expected) = expected {
            if matches!(expected, ir::Type::Function { .. })
                && self.is_callable_reference_expr(expr)
            {
                return Some(self.lower_callable_reference_rvalue(expr, expected));
            }
        }
        if let Some(path) = expr_path(expr) {
            let path = self.canonical_enum_case_path(&path);
            if path.len() > 1 && is_named_runtime_value_path(self.program, &path) {
                let path = self
                    .canonical_declared_type_path(&path)
                    .map(|name| vec![name])
                    .unwrap_or(path);
                return Some(ir::RValue::NamedValue { path });
            }
        }
        match expr {
            Expr::ListLiteral { items, .. } if !list_literal_has_spread(items) => Some(
                ir::RValue::List(items.iter().map(|item| self.lower_expr(item)).collect()),
            ),
            Expr::TupleLiteral { items, .. } => Some(ir::RValue::Tuple(
                items.iter().map(|item| self.lower_expr(item)).collect(),
            )),
            Expr::RecordLiteral { fields, values, .. } => {
                if let Some(expected) = expected {
                    if fields.is_empty() && values.is_empty() && matches!(expected, ir::Type::Unit)
                    {
                        return Some(ir::RValue::Use(ir::Operand::Const(ir::Constant::Unit)));
                    }
                    if fields.is_empty()
                        && values.is_empty()
                        && matches!(expected, ir::Type::Named { name, .. } if runtime_collection_constructor_name(name))
                    {
                        let ir::Type::Named { name, .. } = expected else {
                            unreachable!()
                        };
                        let span = expr.span();
                        let call = Expr::Call {
                            callee: Box::new(Expr::Identifier {
                                name: name.clone(),
                                span,
                            }),
                            args: Vec::new(),
                            style: core::CallStyle::Paren,
                            span,
                        };
                        return self.lower_rvalue_with_expected(&call, Some(expected));
                    }
                    if let ir::Type::Named { name, .. } = expected
                        && self.program.types.iter().any(|ty| {
                            ty.name == *name
                                && ty.kind == ast::TypeKind::Class
                                && ty.methods.iter().copied().any(|id| {
                                    self.program
                                        .function(id)
                                        .is_some_and(|function| function.name == "new")
                                })
                        })
                    {
                        let span = expr.span();
                        let call = Expr::Call {
                            callee: Box::new(Expr::Identifier {
                                name: name.clone(),
                                span,
                            }),
                            args: vec![core::CallArg {
                                name: None,
                                ty: None,
                                value: expr.clone(),
                                span,
                            }],
                            style: core::CallStyle::Brace,
                            span,
                        };
                        return self.lower_rvalue_with_expected(&call, Some(expected));
                    }
                    if let Some(value) =
                        self.lower_record_literal_as_named_construct(fields, values, expected)
                    {
                        return Some(value);
                    }
                }
                if fields.is_empty() && !values.is_empty() {
                    Some(ir::RValue::Tuple(
                        values.iter().map(|value| self.lower_expr(value)).collect(),
                    ))
                } else if fields.iter().any(|field| field.name.is_none()) {
                    Some(ir::RValue::RecordSpread(
                        fields
                            .iter()
                            .map(|field| {
                                if let Some(name) = &field.name {
                                    ir::RecordSpreadPart::Field(ir::NamedOperand {
                                        name: name.clone(),
                                        value: self.lower_expr(&field.value),
                                    })
                                } else {
                                    let Expr::Spread {
                                        value,
                                        override_existing,
                                        ..
                                    } = &field.value
                                    else {
                                        unreachable!(
                                            "shape spread fields are represented by Expr::Spread"
                                        )
                                    };
                                    ir::RecordSpreadPart::Spread {
                                        value: self.lower_expr(value),
                                        override_existing: *override_existing,
                                    }
                                }
                            })
                            .collect(),
                    ))
                } else {
                    Some(ir::RValue::Record(
                        fields
                            .iter()
                            .map(|field| ir::NamedOperand {
                                name: field.name.clone().unwrap_or_default(),
                                value: self.lower_expr(&field.value),
                            })
                            .collect(),
                    ))
                }
            }
            Expr::Unary {
                op: ast::UnaryOp::OptionWrap,
                expr,
                ..
            } => {
                let payload_ty = match expected {
                    Some(ir::Type::Named { name, args }) if name == "Option" && args.len() == 1 => {
                        args[0].clone()
                    }
                    _ => self.infer_expr_type(expr),
                };
                Some(ir::RValue::Variant {
                    enum_name: "Option".to_string(),
                    case_name: "Some".to_string(),
                    fields: vec![ir::NamedOperand {
                        name: "value".to_string(),
                        value: self.lower_expr_with_expected(expr, Some(&payload_ty)),
                    }],
                })
            }
            Expr::Unary {
                op: ast::UnaryOp::UnsafeExtract,
                expr,
                ..
            } => Some(ir::RValue::Call {
                callee: ir::Callee::Intrinsic(ir::Intrinsic::UnsafeExtractSuccessValue),
                args: vec![self.lower_expr(expr)],
                structural: false,
            }),
            Expr::Unary { op, expr, .. } => Some(ir::RValue::Unary {
                op: match op {
                    ast::UnaryOp::Neg => ir::UnaryOp::Neg,
                    ast::UnaryOp::Not => ir::UnaryOp::Not,
                    ast::UnaryOp::OptionWrap => unreachable!(),
                    ast::UnaryOp::UnsafeExtract => unreachable!(),
                },
                operand: self.lower_expr(expr),
            }),
            Expr::Binary {
                left, op, right, ..
            } => Some(self.lower_binary_rvalue(left, *op, right)),
            Expr::Call {
                callee,
                args,
                style,
                span,
            } => {
                if let Expr::Member { receiver, name, .. } = callee.as_ref()
                    && name == "equals"
                    && args.len() == 1
                    && self
                        .shape_equality_fields(&self.infer_expr_type(receiver))
                        .is_some()
                    && self
                        .shape_equality_fields(&self.infer_expr_type(&args[0].value))
                        .is_some()
                {
                    return Some(self.lower_shape_equality_rvalue(
                        receiver,
                        AstBinaryOp::Eq,
                        &args[0].value,
                    ));
                }
                let (candidate_callee, candidate_type_args) =
                    self.split_generic_call_callee(callee);
                let candidate_normalized_args =
                    self.normalize_trailing_brace_call_args(candidate_callee, args, *style);
                let use_generic_callee = !candidate_type_args.is_empty()
                    && (self
                        .reified_call_target(candidate_callee, &candidate_normalized_args)
                        .and_then(|(function, _)| self.program.function(function))
                        .is_some_and(|function| !function.type_params.is_empty())
                        || self.is_builtin_reified_metadata_call(candidate_callee)
                        || expr_path(candidate_callee).is_some_and(|path| {
                            path.len() == 1
                                && (declared_type_exists(self.program, &path[0])
                                    || runtime_collection_constructor_name(&path[0]))
                        }));
                let (call_callee, explicit_type_args, normalized_args) = if use_generic_callee {
                    (
                        candidate_callee,
                        candidate_type_args,
                        candidate_normalized_args,
                    )
                } else {
                    (
                        callee.as_ref(),
                        Vec::new(),
                        self.normalize_trailing_brace_call_args(callee, args, *style),
                    )
                };
                let expected_args =
                    self.call_expected_arg_specs(call_callee, &normalized_args, expected);
                // Evaluate the callee (including a method receiver) before any explicit
                // arguments. Argument binding order is handled separately below.
                let lowered_callee = self.lower_callee(call_callee);
                let (ordered_args, mut lowered_args) = expected_args
                    .as_deref()
                    .and_then(|specs| self.lower_call_args_with_defaults(&normalized_args, specs))
                    .unwrap_or_else(|| {
                        let ordered = self
                            .reorder_call_args(call_callee, &normalized_args)
                            .into_iter()
                            .collect::<Vec<_>>();
                        let lowered = self.lower_reordered_call_args_with_spread(
                            &normalized_args,
                            &ordered,
                            expected_args.as_deref(),
                        );
                        let ordered = ordered.into_iter().cloned().collect::<Vec<_>>();
                        (ordered, lowered)
                    });
                let reified_args = self.reified_call_evidence_args(
                    call_callee,
                    &ordered_args,
                    &explicit_type_args,
                );
                let builtin_reified_args =
                    self.builtin_reified_metadata_evidence_args(call_callee, &explicit_type_args);
                let reified_arg_count = reified_args.len() + builtin_reified_args.len();
                lowered_args.extend(reified_args);
                lowered_args.extend(builtin_reified_args);
                self.program.source_calls.push(ir::SourceCall {
                    function: self.function_id,
                    span: *span,
                    callee: lowered_callee.clone(),
                    lowered_args: lowered_args.clone(),
                    ordered_arg_spans: ordered_args.iter().map(|arg| arg.span).collect(),
                    reified_arg_count,
                    param_specs: expected_args
                        .unwrap_or_default()
                        .into_iter()
                        .map(|spec| {
                            spec.map(|spec| ir::SourceCallParamSpec {
                                name: spec.name,
                                ty: spec.ty,
                                lazy: spec.lazy,
                                variadic: spec.variadic,
                                default: spec.default,
                            })
                        })
                        .collect(),
                });
                Some(ir::RValue::Call {
                    callee: lowered_callee,
                    args: lowered_args,
                    structural: call_uses_structural_record_arg(&normalized_args, *style),
                })
            }
            Expr::ContextualNew { args, style, span } => {
                if *style == core::CallStyle::Brace
                    && expected.is_none_or(|ty| {
                        self.named_construct_fields(ty).is_none()
                            && !matches!(
                                ty,
                                ir::Type::Named { name, .. }
                                    if runtime_collection_constructor_name(name)
                            )
                    })
                {
                    let [
                        core::CallArg {
                            value: record @ Expr::RecordLiteral { .. },
                            ..
                        },
                    ] = args.as_slice()
                    else {
                        self.invariant(
                            "'new { ... }' should contain one record literal before lowering",
                            *span,
                        );
                        return Some(ir::RValue::Use(ir::Operand::Const(ir::Constant::Unit)));
                    };
                    return self.lower_rvalue_with_expected(record, expected);
                }
                let Some(ir::Type::Named { name, .. }) = expected else {
                    self.invariant(
                        "contextual 'new(...)' should have an expected named class or shape before lowering",
                        *span,
                    );
                    return Some(ir::RValue::Use(ir::Operand::Const(ir::Constant::Unit)));
                };
                let call = Expr::Call {
                    callee: Box::new(Expr::Identifier {
                        name: name.clone(),
                        span: *span,
                    }),
                    args: args.clone(),
                    style: *style,
                    span: *span,
                };
                self.lower_rvalue_with_expected(&call, expected)
            }
            Expr::Member { receiver, name, .. }
                if name == "args"
                    && matches!(receiver.as_ref(), Expr::Identifier { name, .. } if name == "OS") =>
            {
                Some(ir::RValue::Call {
                    callee: ir::Callee::Intrinsic(ir::Intrinsic::ProgramArgs),
                    args: Vec::new(),
                    structural: false,
                })
            }
            Expr::Member { receiver, name, .. } => {
                let receiver_ty = self.infer_expr_type(receiver);
                if self.is_getter_member_for_type(&receiver_ty, name) {
                    Some(ir::RValue::Call {
                        callee: ir::Callee::Method {
                            receiver: self.lower_expr(receiver),
                            method: name.clone(),
                        },
                        args: Vec::new(),
                        structural: false,
                    })
                } else {
                    Some(ir::RValue::Field {
                        base: self.lower_expr(receiver),
                        name: name.clone(),
                    })
                }
            }
            Expr::Index {
                receiver, index, ..
            } => Some(ir::RValue::Index {
                base: self.lower_expr(receiver),
                index: self.lower_expr(index),
            }),
            Expr::Is { left, target, .. } => Some(ir::RValue::TypeTest {
                operand: self.lower_expr(left),
                ty: self.lower_type_test_target(target),
            }),
            Expr::TypeOf { ty, .. } => {
                if let Some(operand) = self.reified_type_param_operand(ty) {
                    Some(ir::RValue::Use(operand))
                } else {
                    Some(ir::RValue::TypeOf {
                        ty: self.lower_type_ref(ty),
                    })
                }
            }
            Expr::Lambda { params, body, span } => {
                let (expected_params, expected_return) = match expected {
                    Some(ir::Type::Function { params, ret }) => {
                        (Some(params.as_slice()), Some(ret.as_ref().clone()))
                    }
                    _ => (None, None),
                };
                Some(self.lower_lambda_rvalue(
                    params,
                    body,
                    *span,
                    expected_params,
                    expected_return,
                ))
            }
            Expr::AnonymousObject {
                kind,
                interfaces,
                fields,
                methods,
                span,
            } => {
                Some(self.lower_anonymous_object_rvalue(*kind, interfaces, fields, methods, *span))
            }
            Expr::RecordUpdate {
                receiver, patch, ..
            } => Some(ir::RValue::RecordUpdate {
                base: self.lower_expr(receiver),
                patch: self.lower_expr(patch),
            }),
            Expr::Placeholder { .. } => None,
            _ => None,
        }
    }

    fn lower_call_arg(
        &mut self,
        arg: &core::CallArg,
        expected: Option<&ExpectedArgSpec>,
    ) -> ir::Operand {
        let value = match &arg.value {
            Expr::Spread { value, .. } => value.as_ref(),
            _ => &arg.value,
        };
        let Some(expected) = expected else {
            return self.lower_expr(value);
        };
        if expected.lazy {
            if let Some(thunk) = self.lazy_forward_operand(value) {
                return thunk;
            }
            return self.lower_lazy_argument(value, expected.ty.clone(), arg.span);
        }
        self.lower_expr_with_expected(value, Some(&expected.ty))
    }

    fn lower_reordered_call_args_with_spread(
        &mut self,
        source_args: &[core::CallArg],
        ordered_args: &[&core::CallArg],
        expected: Option<&[Option<ExpectedArgSpec>]>,
    ) -> Vec<ir::Operand> {
        let ordered_positions = source_args
            .iter()
            .map(|source| {
                ordered_args
                    .iter()
                    .position(|ordered| std::ptr::eq(source, *ordered))
            })
            .collect::<Vec<_>>();
        let variadic_index = expected.and_then(|specs| {
            specs
                .iter()
                .position(|spec| spec.as_ref().is_some_and(|spec| spec.variadic))
        });
        let variadic_spec = variadic_index.and_then(|index| {
            expected
                .and_then(|specs| specs.get(index))
                .and_then(Option::as_ref)
                .cloned()
        });
        let variadic_sources = ordered_positions
            .iter()
            .enumerate()
            .filter_map(|(source_index, position)| {
                position
                    .is_some_and(|position| {
                        variadic_index.is_some_and(|variadic| position >= variadic)
                    })
                    .then_some(source_index)
            })
            .collect::<Vec<_>>();
        let pack_variadic = variadic_index.is_some()
            && variadic_sources
                .iter()
                .any(|index| matches!(source_args[*index].value, Expr::Spread { .. }));
        let variadic_is_named_value = variadic_sources.len() == 1
            && source_args
                .get(variadic_sources[0])
                .is_some_and(|arg| arg.name.is_some());
        let element_ty = variadic_spec
            .as_ref()
            .map(|spec| match &spec.ty {
                ir::Type::Named { name, args } if name == "Vector" && args.len() == 1 => {
                    args[0].clone()
                }
                _ => ir::Type::Unknown,
            })
            .unwrap_or(ir::Type::Unknown);
        let element_spec = ExpectedArgSpec {
            name: None,
            ty: element_ty.clone(),
            lazy: false,
            variadic: false,
            default: None,
        };
        let mut lowered_by_source = vec![None; source_args.len()];
        let mut packed_variadic = None;

        for (source_index, arg) in source_args.iter().enumerate() {
            let position = ordered_positions[source_index];
            let is_variadic = position.is_some_and(|position| {
                variadic_index.is_some_and(|variadic| position >= variadic)
            });
            if pack_variadic && is_variadic {
                let list = *packed_variadic.get_or_insert_with(|| {
                    let list = self.add_temp(
                        variadic_spec
                            .as_ref()
                            .map(|spec| spec.ty.clone())
                            .unwrap_or_else(|| ir::Type::list(ir::Type::Unknown)),
                    );
                    self.push_statement(ir::Statement {
                        span: Some(arg.span),
                        kind: ir::StatementKind::Assign {
                            target: ir::Place::Local(list),
                            value: ir::RValue::List(Vec::new()),
                        },
                    });
                    list
                });
                let (intrinsic, value, span) = match &arg.value {
                    Expr::Spread { value, span, .. } => {
                        (ir::Intrinsic::ListExtend, self.lower_expr(value), *span)
                    }
                    _ => (
                        ir::Intrinsic::ListAppend,
                        self.lower_expr_with_expected(&arg.value, Some(&element_ty)),
                        arg.span,
                    ),
                };
                self.push_statement(ir::Statement {
                    span: Some(span),
                    kind: ir::StatementKind::Assign {
                        target: ir::Place::Local(list),
                        value: ir::RValue::Call {
                            callee: ir::Callee::Intrinsic(intrinsic),
                            args: vec![ir::Operand::Copy(Box::new(ir::Place::Local(list))), value],
                            structural: false,
                        },
                    },
                });
                continue;
            }

            let spec = if is_variadic && !variadic_is_named_value {
                Some(&element_spec)
            } else {
                position
                    .and_then(|position| expected.and_then(|specs| specs.get(position)))
                    .and_then(Option::as_ref)
            };
            lowered_by_source[source_index] = Some(self.lower_call_arg(arg, spec));
        }

        let mut out = Vec::new();
        let mut emitted_packed_variadic = false;
        for (position, arg) in ordered_args.iter().enumerate() {
            if pack_variadic && variadic_index.is_some_and(|variadic| position >= variadic) {
                if !emitted_packed_variadic {
                    let list =
                        packed_variadic.expect("variadic source initialized its packed list");
                    out.push(ir::Operand::Copy(Box::new(ir::Place::Local(list))));
                    emitted_packed_variadic = true;
                }
                continue;
            }
            let source_index = source_args
                .iter()
                .position(|source| std::ptr::eq(source, *arg))
                .expect("ordered call argument came from the source call");
            out.push(
                lowered_by_source[source_index]
                    .clone()
                    .expect("source call argument was lowered"),
            );
        }
        out
    }

    fn lower_call_args_with_defaults(
        &mut self,
        args: &[core::CallArg],
        expected: &[Option<ExpectedArgSpec>],
    ) -> Option<(Vec<core::CallArg>, Vec<ir::Operand>)> {
        let specs = expected.iter().cloned().collect::<Option<Vec<_>>>()?;
        let mut slots = vec![Vec::<usize>::new(); specs.len()];
        let mut assigned_specs = vec![None; args.len()];
        let mut positional_index = 0usize;

        for (source_index, arg) in args.iter().enumerate() {
            if let Some(name) = &arg.name {
                let index = specs
                    .iter()
                    .position(|spec| spec.name.as_deref() == Some(name.as_str()))?;
                if !slots[index].is_empty() {
                    return None;
                }
                slots[index].push(source_index);
                assigned_specs[source_index] = Some(index);
                continue;
            }

            while positional_index < specs.len()
                && !specs[positional_index].variadic
                && !slots[positional_index].is_empty()
            {
                positional_index += 1;
            }
            if specs.last().is_some_and(|spec| spec.variadic)
                && positional_index >= specs.len().saturating_sub(1)
            {
                slots.last_mut()?.push(source_index);
                assigned_specs[source_index] = Some(specs.len().saturating_sub(1));
            } else if positional_index < specs.len() {
                slots[positional_index].push(source_index);
                assigned_specs[source_index] = Some(positional_index);
                positional_index += 1;
            } else {
                return None;
            }
        }

        let ordered_args = slots
            .iter()
            .flat_map(|slot| slot.iter().map(|index| args[*index].clone()))
            .collect::<Vec<_>>();
        let variadic_index = specs.iter().position(|spec| spec.variadic);
        let variadic_sources = variadic_index
            .map(|index| slots[index].clone())
            .unwrap_or_default();
        let pack_variadic = variadic_index.is_some()
            && variadic_sources
                .iter()
                .any(|index| matches!(args[*index].value, Expr::Spread { .. }));
        let variadic_is_named_value =
            variadic_sources.len() == 1 && args[variadic_sources[0]].name.is_some();
        let element_ty = variadic_index
            .map(|index| match &specs[index].ty {
                ir::Type::Named { name, args } if name == "Vector" && args.len() == 1 => {
                    args[0].clone()
                }
                _ => ir::Type::Unknown,
            })
            .unwrap_or(ir::Type::Unknown);
        let element_spec = ExpectedArgSpec {
            name: None,
            ty: element_ty.clone(),
            lazy: false,
            variadic: false,
            default: None,
        };
        let mut lowered_by_source = vec![None; args.len()];
        let mut packed_variadic = None;

        for (source_index, arg) in args.iter().enumerate() {
            let spec_index = assigned_specs[source_index]?;
            if Some(spec_index) == variadic_index && pack_variadic {
                let list = *packed_variadic.get_or_insert_with(|| {
                    let list = self.add_temp(specs[spec_index].ty.clone());
                    self.push_statement(ir::Statement {
                        span: Some(arg.span),
                        kind: ir::StatementKind::Assign {
                            target: ir::Place::Local(list),
                            value: ir::RValue::List(Vec::new()),
                        },
                    });
                    list
                });
                let (intrinsic, value, span) = match &arg.value {
                    Expr::Spread { value, span, .. } => {
                        (ir::Intrinsic::ListExtend, self.lower_expr(value), *span)
                    }
                    _ => (
                        ir::Intrinsic::ListAppend,
                        self.lower_expr_with_expected(&arg.value, Some(&element_ty)),
                        arg.span,
                    ),
                };
                self.push_statement(ir::Statement {
                    span: Some(span),
                    kind: ir::StatementKind::Assign {
                        target: ir::Place::Local(list),
                        value: ir::RValue::Call {
                            callee: ir::Callee::Intrinsic(intrinsic),
                            args: vec![ir::Operand::Copy(Box::new(ir::Place::Local(list))), value],
                            structural: false,
                        },
                    },
                });
                continue;
            }
            let spec = if Some(spec_index) == variadic_index && !variadic_is_named_value {
                &element_spec
            } else {
                &specs[spec_index]
            };
            lowered_by_source[source_index] = Some(self.lower_call_arg(arg, Some(spec)));
        }

        let mut lowered = Vec::new();
        for (index, (slot, spec)) in slots.into_iter().zip(specs.iter()).enumerate() {
            if spec.variadic {
                if slot.is_empty() {
                    continue;
                }
                if pack_variadic {
                    let list =
                        packed_variadic.expect("variadic source initialized its packed list");
                    lowered.push(ir::Operand::Copy(Box::new(ir::Place::Local(list))));
                } else {
                    lowered.extend(slot.into_iter().map(|source_index| {
                        lowered_by_source[source_index]
                            .clone()
                            .expect("variadic source argument was lowered")
                    }));
                }
                continue;
            }
            match slot.as_slice() {
                [source_index] => lowered.push(
                    lowered_by_source[*source_index]
                        .clone()
                        .expect("source call argument was lowered"),
                ),
                [] => lowered.push(ir::Operand::Const(spec.default.clone()?)),
                _ => return None,
            }
            debug_assert_ne!(Some(index), variadic_index);
        }
        Some((ordered_args, lowered))
    }

    fn lazy_forward_operand(&mut self, expr: &Expr) -> Option<ir::Operand> {
        let Expr::Identifier { name, .. } = expr else {
            return None;
        };
        self.lazy_values
            .contains_key(name)
            .then(|| self.lookup_scoped_or_captured_value(name))
            .flatten()
    }

    fn lower_lazy_argument(&mut self, expr: &Expr, return_ty: ir::Type, span: Span) -> ir::Operand {
        let rvalue = self.lower_lazy_argument_closure(expr, return_ty.clone(), span);
        self.emit_temp_from_rvalue(rvalue, lazy_storage_type(return_ty), Some(span))
    }

    fn lower_lazy_argument_closure(
        &mut self,
        expr: &Expr,
        return_ty: ir::Type,
        span: Span,
    ) -> ir::RValue {
        let nested_name = format!(
            "lazy${}${}",
            self.function_id.0,
            self.function().blocks.len()
        );
        let mut nested = ir::Function::new(nested_name, ir::FunctionKind::Lambda, return_ty);
        nested.span = Some(span);
        let function_id = self.program.add_function(nested);
        let capture_sources = self.visible_capture_sources(None);
        let captures = {
            let mut lowerer = FunctionLowerer::new(
                self.program,
                self.core_bodies,
                function_id,
                self.globals,
                self.functions,
                self.case_fields,
                self.type_aliases,
                self.diagnostics,
            )
            .with_capture_sources(capture_sources);
            lowerer.lower_callable_body(&CallableBody::Expr(expr.clone()), span);
            lowerer.finish_closure_captures()
        };
        ir::RValue::Closure {
            function: function_id,
            captures,
        }
    }

    fn call_expected_arg_specs(
        &self,
        callee: &Expr,
        ordered_args: &[core::CallArg],
        expected: Option<&ir::Type>,
    ) -> Option<Vec<Option<ExpectedArgSpec>>> {
        if let Some(fields) =
            expected.and_then(|expected| self.enum_case_expected_arg_types(callee, expected))
        {
            return Some(
                fields
                    .into_iter()
                    .map(|ty| {
                        Some(ExpectedArgSpec {
                            name: None,
                            ty,
                            lazy: false,
                            variadic: false,
                            default: None,
                        })
                    })
                    .collect(),
            );
        }

        match callee {
            Expr::Identifier { name, .. } => {
                if name == "ensure" && ordered_args.len() == 2 {
                    let error_ty = match expected {
                        Some(ir::Type::Named { name, args })
                            if name == "Result" && args.len() == 2 =>
                        {
                            args[1].clone()
                        }
                        _ => ir::Type::Unknown,
                    };
                    return Some(vec![
                        Some(ExpectedArgSpec {
                            name: None,
                            ty: ir::Type::Bool,
                            lazy: false,
                            variadic: false,
                            default: None,
                        }),
                        Some(ExpectedArgSpec {
                            name: None,
                            ty: error_ty,
                            lazy: true,
                            variadic: false,
                            default: None,
                        }),
                    ]);
                }
                if let Some(id) = self
                    .functions
                    .get(name)
                    .copied()
                    .or_else(|| self.current_owner_method(name, ordered_args))
                {
                    let mut subst = HashMap::new();
                    if let Some(expected) = expected
                        && let Some(function) = self.program.function(id)
                    {
                        infer_ir_type_subst(&function.return_ty, expected, &mut subst);
                    }
                    return self.function_expected_arg_specs(id, &subst);
                }
                if let Some(specs) = self.constructor_expected_arg_specs(name, ordered_args) {
                    return Some(specs);
                }
                if let Some(specs) = self.enum_case_expected_arg_specs(None, name) {
                    return Some(specs);
                }
            }
            Expr::Member { receiver, name, .. } => {
                if name == "transactionally" && ordered_args.len() == 1 {
                    let value_ty =
                        transactionally_result_value_type(expected).unwrap_or(ir::Type::Unknown);
                    return Some(vec![Some(ExpectedArgSpec {
                        name: None,
                        ty: transactional_work_type(value_ty),
                        lazy: false,
                        variadic: false,
                        default: None,
                    })]);
                }
                let receiver_ty = self.infer_expr_type(receiver);
                if matches!(name.as_str(), "fold" | "reduce") && ordered_args.len() == 2 {
                    if let ir::Type::Named { name, args } = &receiver_ty {
                        if matches!(name.as_str(), "Vector" | "LinkedList") && args.len() == 1 {
                            let accumulator = self.infer_expr_type(&ordered_args[0].value);
                            return Some(vec![
                                Some(ExpectedArgSpec {
                                    name: None,
                                    ty: accumulator.clone(),
                                    lazy: false,
                                    variadic: false,
                                    default: None,
                                }),
                                Some(ExpectedArgSpec {
                                    name: None,
                                    ty: ir::Type::Function {
                                        params: vec![accumulator.clone(), args[0].clone()],
                                        ret: Box::new(accumulator),
                                    },
                                    lazy: false,
                                    variadic: false,
                                    default: None,
                                }),
                            ]);
                        }
                    }
                }
                if let Some((id, mut subst)) =
                    self.method_expected_arg_target(&receiver_ty, name, ordered_args)
                {
                    if let Some(expected) = expected
                        && let Some(function) = self.program.function(id)
                    {
                        infer_ir_type_subst(&function.return_ty, expected, &mut subst);
                    }
                    return self.function_expected_arg_specs(id, &subst);
                }
                if let Some(specs) = builtin_member_expected_arg_specs(&receiver_ty, name, expected)
                {
                    return Some(specs);
                }
            }
            _ => {}
        }

        if let Some(path) = expr_path(callee) {
            if path.len() == 2 {
                let owner = &path[0];
                let member = &path[1];
                if let Some(specs) = self.enum_case_expected_arg_specs(Some(owner), member) {
                    return Some(specs);
                }
                if let Some((id, subst)) = self.named_method_expected_arg_target(
                    owner,
                    ast::TypeKind::Object,
                    member,
                    ordered_args,
                ) {
                    return self.function_expected_arg_specs(id, &subst);
                }
            }
        }

        None
    }

    fn constructor_expected_arg_specs(
        &self,
        name: &str,
        args: &[core::CallArg],
    ) -> Option<Vec<Option<ExpectedArgSpec>>> {
        let ty = self.program.types.iter().find(|ty| {
            ty.name == name && matches!(ty.kind, ast::TypeKind::Class | ast::TypeKind::Record)
        })?;
        let explicit = ty
            .methods
            .iter()
            .copied()
            .filter_map(|id| {
                let function = self.program.function(id)?;
                (function.name == "new").then_some((id, function))
            })
            .filter_map(|(id, function)| {
                method_call_arity_score(function, args).map(|score| (score, id))
            })
            .max_by_key(|(score, _)| *score)
            .map(|(_, id)| id);
        if let Some(id) = explicit {
            return self.function_expected_arg_specs(id, &HashMap::new());
        }
        if ty.methods.iter().copied().any(|id| {
            self.program
                .function(id)
                .is_some_and(|function| function.name == "new")
        }) {
            return None;
        }

        Some(
            ty.fields
                .iter()
                .filter(|field| {
                    if ty.kind == ast::TypeKind::Class {
                        field.visibility == ast::Visibility::Default
                    } else {
                        field.visibility != ast::Visibility::Private
                    }
                })
                .map(|field| {
                    Some(ExpectedArgSpec {
                        name: Some(field.name.clone()),
                        ty: field.ty.clone(),
                        lazy: false,
                        variadic: false,
                        default: field.initializer.clone(),
                    })
                })
                .collect(),
        )
    }

    fn enum_case_expected_arg_specs(
        &self,
        owner: Option<&str>,
        case_name: &str,
    ) -> Option<Vec<Option<ExpectedArgSpec>>> {
        let mut matches = self.program.types.iter().filter(|ty| {
            ty.kind == ast::TypeKind::Enum
                && owner.is_none_or(|owner| ty.name == owner)
                && ty.enum_cases.iter().any(|case| case.name == case_name)
        });
        let ty = matches.next()?;
        if owner.is_none() && matches.next().is_some() {
            return None;
        }
        let case = ty.enum_cases.iter().find(|case| case.name == case_name)?;
        Some(
            case.fields
                .iter()
                .map(|field| {
                    Some(ExpectedArgSpec {
                        name: Some(field.name.clone()),
                        ty: field.ty.clone(),
                        lazy: false,
                        variadic: false,
                        default: field.initializer.clone(),
                    })
                })
                .collect(),
        )
    }

    fn function_expected_arg_specs(
        &self,
        id: ir::FunctionId,
        subst: &HashMap<String, ir::Type>,
    ) -> Option<Vec<Option<ExpectedArgSpec>>> {
        let function = self.program.function(id)?;
        Some(
            source_param_indices(function)
                .into_iter()
                .map(|index| {
                    let local = function
                        .params
                        .get(index)
                        .and_then(|param| function.locals.get(param.0))?;
                    let lazy = function.param_lazy.get(index).copied().unwrap_or(false);
                    let ty = if lazy {
                        lazy_value_type(&local.ty).unwrap_or(ir::Type::Unknown)
                    } else {
                        local.ty.clone()
                    };
                    Some(ExpectedArgSpec {
                        name: Some(local.name.clone()),
                        ty: substitute_ir_type(&ty, subst),
                        lazy,
                        variadic: function.param_variadic.get(index).copied().unwrap_or(false),
                        default: function.param_defaults.get(index).cloned().flatten(),
                    })
                })
                .collect(),
        )
    }

    fn method_expected_arg_target(
        &self,
        receiver_ty: &ir::Type,
        method: &str,
        args: &[core::CallArg],
    ) -> Option<(ir::FunctionId, HashMap<String, ir::Type>)> {
        let ir::Type::Named {
            name: type_name,
            args: type_args,
        } = receiver_ty
        else {
            return None;
        };
        let ty = self.program.types.iter().find(|ty| ty.name == *type_name)?;
        let mut subst = ir_type_subst(ty, type_args);
        let id = self.find_method_expected_arg_target(ty, method, args)?;
        self.infer_function_call_subst(id, args, &mut subst);
        Some((id, subst))
    }

    fn named_method_expected_arg_target(
        &self,
        owner: &str,
        kind: ast::TypeKind,
        method: &str,
        args: &[core::CallArg],
    ) -> Option<(ir::FunctionId, HashMap<String, ir::Type>)> {
        let ty = self
            .program
            .types
            .iter()
            .find(|ty| ty.name == owner && ty.kind == kind)?;
        let id = self.find_method_expected_arg_target(ty, method, args)?;
        let mut subst = HashMap::new();
        self.infer_function_call_subst(id, args, &mut subst);
        Some((id, subst))
    }

    fn find_method_expected_arg_target(
        &self,
        ty: &ir::TypeDef,
        method: &str,
        args: &[core::CallArg],
    ) -> Option<ir::FunctionId> {
        let mut best: Option<(usize, ir::FunctionId)> = None;
        for method_id in &ty.methods {
            let Some(function) = self.program.function(*method_id) else {
                continue;
            };
            if function.name != method {
                continue;
            }
            let Some(score) = method_call_arity_score(function, args) else {
                continue;
            };
            if best
                .as_ref()
                .map(|(best_score, _)| score > *best_score)
                .unwrap_or(true)
            {
                best = Some((score, *method_id));
            }
        }
        best.map(|(_, id)| id)
    }

    fn enum_case_expected_arg_types(
        &self,
        callee: &Expr,
        expected: &ir::Type,
    ) -> Option<Vec<ir::Type>> {
        let ir::Type::Named {
            name: expected_name,
            args,
        } = expected
        else {
            return None;
        };
        let path = self.canonical_enum_case_path(&expr_path(callee)?);
        let case_name = match path.as_slice() {
            [case] => case,
            [owner, case] if owner == expected_name => case,
            _ => return None,
        };
        match (expected_name.as_str(), case_name.as_str(), args.as_slice()) {
            ("Option", "Some", [value]) => return Some(vec![value.clone()]),
            ("Result", "Ok", [value, _]) => return Some(vec![value.clone()]),
            ("Result", "Err", [_, error]) => return Some(vec![error.clone()]),
            ("Either", "Left", [left, _]) => return Some(vec![left.clone()]),
            ("Either", "Right", [_, right]) => return Some(vec![right.clone()]),
            _ => {}
        }
        let ty = self
            .program
            .types
            .iter()
            .find(|ty| ty.name == *expected_name && ty.kind == ast::TypeKind::Enum)?;
        let enum_case = ty
            .enum_cases
            .iter()
            .find(|enum_case| enum_case.name == *case_name)?;
        let subst = ty
            .type_params
            .iter()
            .cloned()
            .zip(args.iter().cloned())
            .collect::<HashMap<_, _>>();
        Some(
            enum_case
                .fields
                .iter()
                .map(|field| substitute_ir_type(&field.ty, &subst))
                .collect(),
        )
    }

    fn lower_record_literal_as_named_construct(
        &mut self,
        fields: &[core::CallArg],
        values: &[Expr],
        expected: &ir::Type,
    ) -> Option<ir::RValue> {
        if !values.is_empty() {
            return None;
        }
        let expected_fields = self.named_construct_fields(expected)?;
        if fields
            .iter()
            .filter(|field| field.name.is_some())
            .any(|field| {
                field
                    .name
                    .as_ref()
                    .is_some_and(|name| !expected_fields.iter().any(|expected| expected.0 == *name))
            })
        {
            return None;
        }

        // Evaluate every supplied field and spread once in literal order. The
        // target's declaration order only controls where those values are stored.
        let mut explicit_fields = Vec::new();
        let mut spread_sources = Vec::new();
        for (source_index, field) in fields.iter().enumerate() {
            if let Some(name) = &field.name {
                let ty = expected_fields
                    .iter()
                    .find(|expected| expected.0 == *name)
                    .map(|expected| &expected.1)?;
                explicit_fields.push((
                    source_index,
                    name.clone(),
                    self.lower_expr_with_expected(&field.value, Some(ty)),
                ));
                continue;
            }
            let Expr::Spread {
                value,
                override_existing,
                ..
            } = &field.value
            else {
                return None;
            };
            let source_fields = self.shape_equality_fields(&self.infer_expr_type(value))?;
            spread_sources.push((
                source_index,
                source_fields,
                self.lower_expr(value),
                *override_existing,
            ));
        }

        let mut lowered_fields = Vec::new();
        for (name, ty, has_initializer, initializer) in expected_fields {
            let value = if let Some((_, _, value)) = explicit_fields
                .iter()
                .find(|(_, field_name, _)| field_name == &name)
            {
                value.clone()
            } else if let Some((_, _, source, _)) =
                spread_sources
                    .iter()
                    .rev()
                    .find(|(_, source_fields, _, _)| {
                        source_fields.iter().any(|field| field.name == name)
                    })
            {
                ir::Operand::Copy(Box::new(ir::Place::Field {
                    base: Box::new(source.clone()),
                    name: name.clone(),
                }))
            } else if has_initializer {
                ir::Operand::Const(initializer.unwrap_or_else(|| default_constant_for_type(&ty)))
            } else {
                return None;
            };
            lowered_fields.push(ir::NamedOperand { name, value });
        }

        Some(ir::RValue::Construct {
            ty: expected.clone(),
            fields: lowered_fields,
        })
    }

    fn named_construct_fields(
        &self,
        expected: &ir::Type,
    ) -> Option<Vec<(String, ir::Type, bool, Option<ir::Constant>)>> {
        let ir::Type::Named { name, args } = expected else {
            return None;
        };
        let ty = self.program.types.iter().find(|ty| {
            ty.name == *name && matches!(ty.kind, ast::TypeKind::Class | ast::TypeKind::Record)
        })?;
        let subst = ty
            .type_params
            .iter()
            .cloned()
            .zip(args.iter().cloned())
            .collect::<HashMap<_, _>>();
        Some(
            ty.fields
                .iter()
                .filter(|field| {
                    if ty.kind == ast::TypeKind::Class {
                        field.visibility == ast::Visibility::Default
                    } else {
                        field.visibility != ast::Visibility::Private
                    }
                })
                .map(|field| {
                    (
                        field.name.clone(),
                        substitute_ir_type(&field.ty, &subst),
                        field.has_initializer,
                        field.initializer.clone(),
                    )
                })
                .collect(),
        )
    }

    fn lower_binary_rvalue(&mut self, left: &Expr, op: AstBinaryOp, right: &Expr) -> ir::RValue {
        match op {
            AstBinaryOp::Colon => {
                ir::RValue::Tuple(vec![self.lower_expr(left), self.lower_expr(right)])
            }
            AstBinaryOp::Eq | AstBinaryOp::NotEq => {
                let left_ty = self.infer_expr_type(left);
                if self.type_uses_declared_equality(&left_ty) {
                    let call = ir::RValue::Call {
                        callee: ir::Callee::Method {
                            receiver: self.lower_expr(left),
                            method: "equals".to_string(),
                        },
                        args: vec![self.lower_expr(right)],
                        structural: false,
                    };
                    if op == AstBinaryOp::NotEq {
                        ir::RValue::Unary {
                            op: ir::UnaryOp::Not,
                            operand: self.emit_temp_from_rvalue(
                                call,
                                ir::Type::Bool,
                                Some(left.span()),
                            ),
                        }
                    } else {
                        call
                    }
                } else {
                    self.lower_shape_equality_rvalue(left, op, right)
                }
            }
            _ => ir::RValue::Binary {
                op: map_binary_op(op).expect("non-special binary operator should map to IR"),
                left: self.lower_expr(left),
                right: self.lower_expr(right),
            },
        }
    }

    fn type_uses_declared_equality(&self, ty: &ir::Type) -> bool {
        match ty {
            ir::Type::Named { name, .. } => self.program.types.iter().any(|candidate| {
                candidate.name == *name
                    && matches!(
                        candidate.kind,
                        ast::TypeKind::Class | ast::TypeKind::Interface
                    )
            }),
            ir::Type::TypeParam(name) => {
                self.function().generic_conditions.iter().any(|condition| {
                    matches!(
                        condition,
                        ir::GenericCondition::Bound {
                            subject: ir::Type::TypeParam(subject),
                            bound: ir::Type::Named { name: bound, .. }
                        } if subject.as_str() == name.as_str() && bound == "Eq"
                    )
                })
            }
            _ => false,
        }
    }

    fn lower_shape_equality_rvalue(
        &mut self,
        left: &Expr,
        op: AstBinaryOp,
        right: &Expr,
    ) -> ir::RValue {
        let left_ty = self.infer_expr_type(left);
        let right_ty = self.infer_expr_type(right);
        let left_operand = self.lower_expr(left);
        let right_operand = self.lower_expr(right);
        let right_operand = if left_ty != right_ty
            && self.shape_equality_fields(&left_ty).is_some()
            && self.shape_equality_fields(&right_ty).is_some()
        {
            self.coerce_shape_equality_operand(right_operand, &left_ty, right.span())
        } else {
            right_operand
        };
        ir::RValue::Binary {
            op: map_binary_op(op).expect("equality operator should map to IR"),
            left: left_operand,
            right: right_operand,
        }
    }

    fn shape_equality_fields(&self, ty: &ir::Type) -> Option<Vec<ir::NamedType>> {
        match ty {
            ir::Type::Record(fields) => Some(fields.clone()),
            ir::Type::Named { name, args } => {
                let shape = self
                    .program
                    .types
                    .iter()
                    .find(|ty| ty.name == *name && ty.kind == ast::TypeKind::Record)?;
                let subst = shape
                    .type_params
                    .iter()
                    .cloned()
                    .zip(args.iter().cloned())
                    .collect::<HashMap<_, _>>();
                Some(
                    shape
                        .fields
                        .iter()
                        .filter(|field| field.visibility != ast::Visibility::Private)
                        .map(|field| ir::NamedType {
                            name: field.name.clone(),
                            ty: substitute_ir_type(&field.ty, &subst),
                        })
                        .collect(),
                )
            }
            _ => None,
        }
    }

    fn coerce_shape_equality_operand(
        &mut self,
        source: ir::Operand,
        target: &ir::Type,
        span: Span,
    ) -> ir::Operand {
        let Some(target_fields) = self.shape_equality_fields(target) else {
            return source;
        };
        let mut fields = Vec::with_capacity(target_fields.len());
        for field in target_fields {
            let value = self.emit_temp_from_rvalue(
                ir::RValue::Field {
                    base: source.clone(),
                    name: field.name.clone(),
                },
                field.ty,
                Some(span),
            );
            fields.push(ir::NamedOperand {
                name: field.name,
                value,
            });
        }
        let value = match target {
            ir::Type::Named { .. } => ir::RValue::Construct {
                ty: target.clone(),
                fields,
            },
            ir::Type::Record(_) => ir::RValue::Record(fields),
            _ => return source,
        };
        self.emit_temp_from_rvalue(value, target.clone(), Some(span))
    }

    fn lower_callee(&mut self, callee: &Expr) -> ir::Callee {
        if let Expr::Member { receiver, name, .. } = callee
            && !self.callee_receiver_is_static_namespace(receiver)
        {
            let receiver_ty = self.infer_expr_type(receiver);
            if self.is_getter_member_for_type(&receiver_ty, name)
                && matches!(self.infer_expr_type(callee), ir::Type::Function { .. })
            {
                return ir::Callee::Indirect(self.lower_expr(callee));
            }
            return ir::Callee::Method {
                receiver: self.lower_expr(receiver),
                method: name.clone(),
            };
        }

        if let Some(path) = expr_path(callee) {
            let path = self.canonical_enum_case_path(&path);
            if let Some(name) = self.canonical_declared_type_path(&path) {
                return ir::Callee::Named { path: vec![name] };
            }
            if path.len() == 1 {
                let name = &path[0];
                if name == "this" && self.function().name == "new" {
                    if let ir::FunctionKind::Method { owner } = self.function().kind {
                        if let Some(owner_name) =
                            self.program.types.get(owner.0).map(|ty| ty.name.clone())
                        {
                            return ir::Callee::Named {
                                path: vec![owner_name],
                            };
                        }
                    }
                }
                if let Some(intrinsic) = intrinsic_for_name(name) {
                    return ir::Callee::Intrinsic(intrinsic);
                }
                if let Some(function) = self.functions.get(name).copied() {
                    return ir::Callee::Direct(function);
                }
                if let Some(value) = self.lookup_value(name) {
                    return ir::Callee::Indirect(value);
                }
                if matches!(
                    self.lookup_implicit_getter_type(name),
                    Some(ir::Type::Function { .. })
                ) {
                    return ir::Callee::Indirect(self.lower_expr(callee));
                }
                if let Some(method) = self.lower_implicit_method_callee(name) {
                    return method;
                }
                if let Some(owner) = unique_bare_enum_case_owner(self.program, name) {
                    return ir::Callee::Named {
                        path: vec![owner.to_string(), name.clone()],
                    };
                }
                if is_named_runtime_callee_path(self.program, &path) {
                    return ir::Callee::Named { path };
                }
            }
            if is_named_runtime_callee_path(self.program, &path) {
                return ir::Callee::Named { path };
            }
        }

        if let Expr::Member { receiver, name, .. } = callee {
            let receiver_ty = self.infer_expr_type(receiver);
            if self.is_getter_member_for_type(&receiver_ty, name)
                && matches!(self.infer_expr_type(callee), ir::Type::Function { .. })
            {
                return ir::Callee::Indirect(self.lower_expr(callee));
            }
            return ir::Callee::Method {
                receiver: self.lower_expr(receiver),
                method: name.clone(),
            };
        }

        ir::Callee::Indirect(self.lower_expr(callee))
    }

    fn callee_receiver_is_static_namespace(&self, receiver: &Expr) -> bool {
        let Some(path) = expr_path(receiver) else {
            return false;
        };
        if matches!(path.as_slice(), [owner, stream] if owner == "OS" && matches!(stream.as_str(), "stdout" | "stderr"))
        {
            return true;
        }
        self.canonical_declared_type_path(&path).is_some()
            || declared_type_exists(self.program, &path.join("."))
            || (path.len() == 1
                && (runtime_callable_root_name(&path[0])
                    || matches!(path[0].as_str(), "Int" | "Float" | "Option")
                    || declared_type_exists(self.program, &path[0])
                    || single_type_exists(self.program, &path[0])))
    }

    fn lower_implicit_method_callee(&mut self, name: &str) -> Option<ir::Callee> {
        let owner_type = match self.function().kind {
            ir::FunctionKind::Method { owner } => self.program.types.get(owner.0),
            _ => {
                let this_ty = if let Some(this_local) = self.this_local {
                    self.function()
                        .locals
                        .get(this_local.0)
                        .map(|local| local.ty.clone())
                } else {
                    self.capture_sources
                        .get("this")
                        .map(|source| source.ty.clone())
                }?;
                let ir::Type::Named {
                    name: owner_name, ..
                } = this_ty
                else {
                    return None;
                };
                self.program.types.iter().find(|ty| ty.name == owner_name)
            }
        };
        let owner_type = owner_type?;
        let method_exists = owner_type.methods.iter().any(|method_id| {
            self.program
                .function(*method_id)
                .is_some_and(|method| method.name == name)
        });
        if !method_exists {
            return None;
        }
        let receiver = self.lookup_value("this")?;
        Some(ir::Callee::Method {
            receiver,
            method: name.to_string(),
        })
    }

    fn normalize_trailing_brace_call_args(
        &self,
        callee: &Expr,
        args: &[core::CallArg],
        style: core::CallStyle,
    ) -> Vec<core::CallArg> {
        if style == core::CallStyle::Brace
            && (self.brace_call_targets_explicit_constructor(callee)
                || self.brace_call_targets_implicit_constructor(callee)
                || self.brace_call_targets_current_constructor(callee)
                || self.brace_call_targets_enum_case(callee)
                || self.brace_call_targets_runtime_collection_constructor(callee))
        {
            if let Some(args) = brace_record_constructor_args(args) {
                return args;
            }
        }
        args.to_vec()
    }

    fn brace_call_targets_explicit_constructor(&self, callee: &Expr) -> bool {
        let Some(path) = expr_path(callee) else {
            return false;
        };
        let Some(name) = self.canonical_declared_type_path(&path) else {
            return false;
        };
        self.program
            .types
            .iter()
            .find(|ty| ty.name == name && ty.kind == ast::TypeKind::Class)
            .is_some_and(|ty| {
                ty.methods.iter().copied().any(|id| {
                    self.program
                        .function(id)
                        .is_some_and(|function| function.name == "new")
                })
            })
    }

    fn brace_call_targets_runtime_collection_constructor(&self, callee: &Expr) -> bool {
        let Expr::Identifier { name, .. } = callee else {
            return false;
        };
        runtime_collection_constructor_name(name)
    }

    fn brace_call_targets_implicit_constructor(&self, callee: &Expr) -> bool {
        let Some(path) = expr_path(callee) else {
            return false;
        };
        let Some(name) = self.canonical_declared_type_path(&path) else {
            return false;
        };
        self.program.types.iter().any(|ty| {
            ty.name == name
                && matches!(ty.kind, ast::TypeKind::Class | ast::TypeKind::Record)
                && !ty.methods.iter().copied().any(|id| {
                    self.program
                        .function(id)
                        .is_some_and(|function| function.name == "new")
                })
        })
    }

    fn brace_call_targets_current_constructor(&self, callee: &Expr) -> bool {
        let Some(path) = expr_path(callee) else {
            return false;
        };
        path.len() == 1
            && path[0] == "this"
            && self.function().name == "new"
            && matches!(self.function().kind, ir::FunctionKind::Method { .. })
    }

    fn brace_call_targets_enum_case(&self, callee: &Expr) -> bool {
        let Some(path) = expr_path(callee) else {
            return false;
        };
        let path = self.canonical_enum_case_path(&path);
        match path.as_slice() {
            [case_name] => self.program.types.iter().any(|ty| {
                ty.kind == ast::TypeKind::Enum
                    && ty.enum_cases.iter().any(|case| case.name == *case_name)
            }),
            [type_name, case_name] => self.program.types.iter().any(|ty| {
                ty.kind == ast::TypeKind::Enum
                    && ty.name == *type_name
                    && ty.enum_cases.iter().any(|case| case.name == *case_name)
            }),
            _ => false,
        }
    }

    fn reorder_call_args<'b>(
        &self,
        callee: &Expr,
        args: &'b [core::CallArg],
    ) -> Vec<&'b core::CallArg> {
        if args.iter().all(|arg| arg.name.is_none()) {
            return args.iter().collect();
        }

        self.call_param_names(callee, args)
            .and_then(|param_names| arrange_named_call_args(&param_names, args))
            .unwrap_or_else(|| args.iter().collect())
    }

    fn call_param_names(&self, callee: &Expr, args: &[core::CallArg]) -> Option<Vec<String>> {
        if let Some(path) = expr_path(callee) {
            if path.len() == 1 {
                let name = &path[0];
                if let Some(id) = self.functions.get(name).copied() {
                    return self.function_param_names(id);
                }
                if let Some(type_def) = self.program.types.iter().find(|ty| {
                    ty.name == *name
                        && matches!(
                            ty.kind,
                            ast::TypeKind::Class | ast::TypeKind::Record | ast::TypeKind::Object
                        )
                }) {
                    if let Some(params) = self.constructor_param_names(type_def, args) {
                        return Some(params);
                    }
                }
                for ty in &self.program.types {
                    if ty.kind == ast::TypeKind::Enum {
                        if let Some(case) = ty.enum_cases.iter().find(|case| case.name == *name) {
                            return Some(
                                case.fields.iter().map(|field| field.name.clone()).collect(),
                            );
                        }
                    }
                }
            } else if path.len() == 2 {
                let owner = &path[0];
                let member = &path[1];
                if matches!(
                    (owner.as_str(), member.as_str()),
                    ("Vector", "from") | ("Set", "from")
                ) {
                    return Some(vec!["values".to_string()]);
                }
                if matches!(
                    (owner.as_str(), member.as_str()),
                    ("Int", "parse") | ("Float", "parse")
                ) {
                    return Some(vec!["text".to_string()]);
                }
                if let Some(params) =
                    self.method_param_names_for_kind(owner, ast::TypeKind::Object, member, args)
                {
                    return Some(params);
                }
                if let Some(case) = self
                    .program
                    .types
                    .iter()
                    .filter(|ty| ty.name == *owner && ty.kind == ast::TypeKind::Enum)
                    .flat_map(|ty| ty.enum_cases.iter())
                    .find(|case| case.name == *member)
                {
                    return Some(case.fields.iter().map(|field| field.name.clone()).collect());
                }
                if let Some(params) = self.method_param_names(member, args, Some(owner)) {
                    return Some(params);
                }
            }
        }

        if let Expr::Member { name, receiver, .. } = callee {
            let _ = receiver;
            return self.method_param_names(name, args, None);
        }

        None
    }

    fn function_param_names(&self, id: ir::FunctionId) -> Option<Vec<String>> {
        let function = self.program.function(id)?;
        Some(param_names_from_function(function))
    }

    fn method_param_names(
        &self,
        method: &str,
        args: &[core::CallArg],
        owner_hint: Option<&str>,
    ) -> Option<Vec<String>> {
        self.program
            .types
            .iter()
            .filter(|ty| owner_hint.is_none_or(|hint| ty.name == hint))
            .flat_map(|ty| ty.methods.iter().copied())
            .filter_map(|id| {
                let function = self.program.function(id)?;
                (function.name == method).then_some((id, function))
            })
            .find_map(|(id, function)| {
                let names = param_names_from_function(function);
                let _ = id;
                if arrange_named_call_args(&names, args).is_some() || args.len() == names.len() {
                    Some(names)
                } else {
                    None
                }
            })
    }

    fn method_param_names_for_kind(
        &self,
        owner: &str,
        kind: ast::TypeKind,
        method: &str,
        args: &[core::CallArg],
    ) -> Option<Vec<String>> {
        self.program
            .types
            .iter()
            .filter(|ty| ty.name == owner && ty.kind == kind)
            .flat_map(|ty| ty.methods.iter().copied())
            .filter_map(|id| {
                let function = self.program.function(id)?;
                (function.name == method).then_some(function)
            })
            .find_map(|function| {
                let names = param_names_from_function(function);
                if arrange_named_call_args(&names, args).is_some() || args.len() == names.len() {
                    Some(names)
                } else {
                    None
                }
            })
    }

    fn constructor_param_names(
        &self,
        ty: &ir::TypeDef,
        args: &[core::CallArg],
    ) -> Option<Vec<String>> {
        let mut init_candidates = ty
            .methods
            .iter()
            .copied()
            .filter_map(|id| {
                let function = self.program.function(id)?;
                (function.name == "new").then_some(function)
            })
            .collect::<Vec<_>>();
        init_candidates.sort_by_key(|function| function.params.len());
        let has_explicit_constructor = !init_candidates.is_empty();
        for function in &init_candidates {
            let names = param_names_from_function(function);
            if arrange_named_call_args(&names, args).is_some() {
                return Some(names);
            }
        }
        if !has_explicit_constructor {
            let names = ty
                .fields
                .iter()
                .filter(|field| {
                    if ty.kind == ast::TypeKind::Class {
                        field.visibility == ast::Visibility::Default
                    } else {
                        field.visibility != ast::Visibility::Private
                    }
                })
                .map(|field| field.name.clone())
                .collect::<Vec<_>>();
            if arrange_named_call_args(&names, args).is_some() || args.len() == names.len() {
                return Some(names);
            }
        }
        None
    }

    fn lower_place(&mut self, expr: &Expr) -> Option<ir::Place> {
        match expr {
            Expr::Identifier { name, span } => self
                .lookup_scoped_or_captured_place(name)
                .or_else(|| self.lookup_implicit_field_place(name))
                .or_else(|| self.lookup_global_place(name))
                .or_else(|| {
                    self.invariant(
                        format!(
                            "assignment target '{}' should resolve before lowering",
                            name
                        ),
                        *span,
                    );
                    None
                }),
            Expr::Member { receiver, name, .. } => Some(ir::Place::Field {
                base: Box::new(self.lower_expr(receiver)),
                name: name.clone(),
            }),
            Expr::Index {
                receiver, index, ..
            } => Some(ir::Place::Index {
                base: Box::new(self.lower_expr(receiver)),
                index: Box::new(self.lower_expr(index)),
            }),
            _ => {
                self.invariant(
                    "assignment target should be validated before lowering",
                    expr.span(),
                );
                None
            }
        }
    }

    fn lookup_value(&mut self, name: &str) -> Option<ir::Operand> {
        self.lookup_place(name)
            .map(|place| ir::Operand::Copy(Box::new(place)))
    }

    fn reified_type_param_operand(&mut self, ty: &TypeRef) -> Option<ir::Operand> {
        let TypeRef::Named { name, args, .. } = ty else {
            return None;
        };
        if !args.is_empty()
            || !self
                .function()
                .reified_type_params
                .iter()
                .any(|param| param == name)
        {
            return None;
        }
        self.lookup_value(&reified_type_param_local_name(name))
    }

    fn split_generic_call_callee<'expr>(
        &self,
        callee: &'expr Expr,
    ) -> (&'expr Expr, Vec<ir::Type>) {
        let Expr::Index {
            receiver, index, ..
        } = callee
        else {
            return (callee, Vec::new());
        };
        let Some(type_args) = generic_call_type_arg_refs_from_expr(index) else {
            return (callee, Vec::new());
        };
        (
            receiver.as_ref(),
            type_args.iter().map(|ty| self.lower_type_ref(ty)).collect(),
        )
    }

    fn reified_call_evidence_args(
        &mut self,
        callee: &Expr,
        args: &[core::CallArg],
        explicit_type_args: &[ir::Type],
    ) -> Vec<ir::Operand> {
        let Some((function_id, mut subst)) = self.reified_call_target(callee, args) else {
            return Vec::new();
        };
        let Some(function) = self.program.function(function_id).cloned() else {
            return Vec::new();
        };
        if function.reified_type_params.is_empty() {
            return Vec::new();
        }
        if explicit_type_args.len() == function.type_params.len() {
            for (name, ty) in function.type_params.iter().zip(explicit_type_args.iter()) {
                subst.insert(name.clone(), ty.clone());
            }
        }
        for (arg, param_index) in args.iter().zip(source_param_indices(&function)) {
            let Some(param) = function.params.get(param_index) else {
                continue;
            };
            let Some(local) = function.locals.get(param.0) else {
                continue;
            };
            let expected = substitute_ir_type(&local.ty, &subst);
            let actual = self.infer_expr_type(&arg.value);
            infer_ir_type_subst(&expected, &actual, &mut subst);
        }
        function
            .reified_type_params
            .iter()
            .map(|name| {
                let ty = subst.get(name).cloned().unwrap_or(ir::Type::Unknown);
                self.runtime_type_operand(ty)
            })
            .collect()
    }

    fn builtin_reified_metadata_evidence_args(
        &mut self,
        callee: &Expr,
        explicit_type_args: &[ir::Type],
    ) -> Vec<ir::Operand> {
        if !self.is_builtin_reified_metadata_call(callee) || explicit_type_args.len() != 1 {
            return Vec::new();
        }
        vec![self.runtime_type_operand(explicit_type_args[0].clone())]
    }

    fn is_builtin_reified_metadata_call(&self, callee: &Expr) -> bool {
        let Expr::Member { receiver, name, .. } = callee else {
            return false;
        };
        matches!(name.as_str(), "annotation" | "hasAnnotation")
            && is_annotated_metadata_type(&self.infer_expr_type(receiver))
    }

    fn reified_call_target(
        &self,
        callee: &Expr,
        args: &[core::CallArg],
    ) -> Option<(ir::FunctionId, HashMap<String, ir::Type>)> {
        match callee {
            Expr::Identifier { name, .. } => self
                .functions
                .get(name)
                .copied()
                .or_else(|| self.current_owner_method(name, args))
                .map(|id| (id, HashMap::new())),
            Expr::Member { receiver, name, .. } => {
                let receiver_ty = self.infer_expr_type(receiver);
                let ir::Type::Named {
                    name: type_name,
                    args: type_args,
                } = receiver_ty
                else {
                    return None;
                };
                let ty = self.program.types.iter().find(|ty| ty.name == type_name)?;
                let subst = ir_type_subst(ty, &type_args);
                self.best_method_function(ty, name, args)
                    .map(|id| (id, subst))
            }
            _ => None,
        }
    }

    fn current_owner_method(&self, name: &str, args: &[core::CallArg]) -> Option<ir::FunctionId> {
        let ir::FunctionKind::Method { owner } = self.function().kind else {
            return None;
        };
        let ty = self.program.types.get(owner.0)?;
        self.best_method_function(ty, name, args)
    }

    fn best_method_function(
        &self,
        ty: &ir::TypeDef,
        name: &str,
        args: &[core::CallArg],
    ) -> Option<ir::FunctionId> {
        let mut best: Option<(usize, ir::FunctionId)> = None;
        for method_id in &ty.methods {
            let Some(function) = self.program.function(*method_id) else {
                continue;
            };
            if function.name != name {
                continue;
            }
            let Some(score) = method_call_arity_score(function, args) else {
                continue;
            };
            if best
                .as_ref()
                .map(|(best_score, _)| score > *best_score)
                .unwrap_or(true)
            {
                best = Some((score, *method_id));
            }
        }
        best.map(|(_, id)| id)
    }

    fn runtime_type_operand(&mut self, ty: ir::Type) -> ir::Operand {
        self.emit_temp_from_rvalue(
            ir::RValue::TypeOf { ty: ty.clone() },
            ir_exact_runtime_type(ty),
            None,
        )
    }

    fn lookup_place(&mut self, name: &str) -> Option<ir::Place> {
        self.lookup_scoped_or_captured_place(name)
            .or_else(|| self.lookup_global_place(name))
    }

    fn lookup_scoped_or_captured_value(&mut self, name: &str) -> Option<ir::Operand> {
        self.lookup_scoped_or_captured_place(name)
            .map(|place| ir::Operand::Copy(Box::new(place)))
    }

    fn lookup_scoped_or_captured_place(&mut self, name: &str) -> Option<ir::Place> {
        for scope in self.scopes.iter().rev() {
            if let Some(local) = scope.get(name).copied() {
                return Some(ir::Place::Local(local));
            }
        }
        if let Some(local) = self.capture_local(name) {
            return Some(ir::Place::Local(local));
        }
        None
    }

    fn lookup_global_place(&self, name: &str) -> Option<ir::Place> {
        self.globals.get(name).copied().map(ir::Place::Global)
    }

    fn lookup_implicit_field_place(&mut self, name: &str) -> Option<ir::Place> {
        let this_ty = if let Some(this_local) = self.this_local {
            self.function().locals.get(this_local.0)?.ty.clone()
        } else {
            self.capture_sources.get("this")?.ty.clone()
        };
        let ir::Type::Named {
            name: type_name, ..
        } = this_ty
        else {
            return None;
        };
        self.program
            .types
            .iter()
            .find(|ty| ty.name == type_name && ty.fields.iter().any(|field| field.name == name))?;
        let this_local = if let Some(this_local) = self.this_local {
            this_local
        } else {
            self.capture_local("this")?
        };
        Some(ir::Place::Field {
            base: Box::new(ir::Operand::Copy(Box::new(ir::Place::Local(this_local)))),
            name: name.to_string(),
        })
    }

    fn capture_local(&mut self, name: &str) -> Option<ir::LocalId> {
        if let Some(local) = self.capture_locals.get(name).copied() {
            return Some(local);
        }
        let source = self.capture_sources.get(name).cloned()?;
        let local = self.add_capture(name.to_string(), source.ty);
        self.root_scope().insert(name.to_string(), local);
        if name == "this" {
            self.this_local = Some(local);
        }
        if let Some(value_ty) = source.lazy_value_ty {
            self.lazy_values.insert(name.to_string(), value_ty);
        }
        self.capture_locals.insert(name.to_string(), local);
        self.closure_captures.push(source.operand);
        Some(local)
    }

    fn current_block_mut(&mut self) -> Option<&mut ir::BasicBlock> {
        let current = self.current_block?;
        self.function_mut().block_mut(current)
    }

    fn push_statement(&mut self, statement: ir::Statement) {
        if let Some(block) = self.current_block_mut() {
            block.push(statement);
        }
    }

    fn terminate(&mut self, terminator: ir::Terminator) {
        if let Some(block) = self.current_block_mut() {
            block.set_terminator(terminator);
        }
        self.current_block = None;
    }

    fn block_has_predecessor(&self, target: ir::BlockId) -> bool {
        self.function()
            .blocks
            .iter()
            .any(|block| match &block.terminator.kind {
                ir::TerminatorKind::Goto(dest) => *dest == target,
                ir::TerminatorKind::Branch {
                    then_block,
                    else_block,
                    ..
                } => *then_block == target || *else_block == target,
                ir::TerminatorKind::Switch { arms, default, .. } => {
                    *default == target || arms.iter().any(|arm| arm.target == target)
                }
                _ => false,
            })
    }

    fn current_scope(&mut self) -> &mut HashMap<String, ir::LocalId> {
        if self.scopes.is_empty() {
            self.scopes.push(HashMap::new());
        }
        self.scopes.last_mut().expect("scope")
    }

    fn root_scope(&mut self) -> &mut HashMap<String, ir::LocalId> {
        if self.scopes.is_empty() {
            self.scopes.push(HashMap::new());
        }
        self.scopes.first_mut().expect("root scope")
    }

    fn push_scope(&mut self) {
        self.scopes.push(HashMap::new());
    }

    fn pop_scope(&mut self) {
        self.scopes.pop();
    }

    fn invariant(&mut self, message: impl Into<String>, span: Span) {
        self.diagnostics
            .push(Diagnostic::error("lower_invariant", message, span));
    }

    fn add_error(&mut self, code: &'static str, message: impl Into<String>, span: Span) {
        self.diagnostics
            .push(Diagnostic::error(code, message, span));
    }
}

fn intrinsic_for_name(name: &str) -> Option<ir::Intrinsic> {
    match name {
        "print" => Some(ir::Intrinsic::Print),
        "println" => Some(ir::Intrinsic::Println),
        "printf" => Some(ir::Intrinsic::Printf),
        "panic" => Some(ir::Intrinsic::Panic),
        "assert" => Some(ir::Intrinsic::Assert),
        "ensure" => Some(ir::Intrinsic::Ensure),
        "identity" => Some(ir::Intrinsic::Identity),
        _ => None,
    }
}

fn map_assign_op(op: AssignOp) -> Option<ir::BinaryOp> {
    match op {
        AssignOp::Assign => None,
        AssignOp::Reassign => None,
        AssignOp::AddAssign => Some(ir::BinaryOp::Add),
        AssignOp::SubAssign => Some(ir::BinaryOp::Sub),
        AssignOp::MulAssign => Some(ir::BinaryOp::Mul),
        AssignOp::DivAssign => Some(ir::BinaryOp::Div),
        AssignOp::ModAssign => Some(ir::BinaryOp::Mod),
    }
}

fn map_binary_op(op: AstBinaryOp) -> Option<ir::BinaryOp> {
    match op {
        AstBinaryOp::Or => Some(ir::BinaryOp::Or),
        AstBinaryOp::And => Some(ir::BinaryOp::And),
        AstBinaryOp::Eq => Some(ir::BinaryOp::Eq),
        AstBinaryOp::NotEq => Some(ir::BinaryOp::NotEq),
        AstBinaryOp::StrictEq => Some(ir::BinaryOp::StrictEq),
        AstBinaryOp::StrictNotEq => Some(ir::BinaryOp::StrictNotEq),
        AstBinaryOp::Less => Some(ir::BinaryOp::Less),
        AstBinaryOp::LessEq => Some(ir::BinaryOp::LessEq),
        AstBinaryOp::Greater => Some(ir::BinaryOp::Greater),
        AstBinaryOp::GreaterEq => Some(ir::BinaryOp::GreaterEq),
        AstBinaryOp::Add => Some(ir::BinaryOp::Add),
        AstBinaryOp::Sub => Some(ir::BinaryOp::Sub),
        AstBinaryOp::Mul => Some(ir::BinaryOp::Mul),
        AstBinaryOp::Div => Some(ir::BinaryOp::Div),
        AstBinaryOp::Mod => Some(ir::BinaryOp::Mod),
        AstBinaryOp::Colon => None,
    }
}

fn lower_type_ref_with_aliases(
    reference: &TypeRef,
    type_aliases: &HashMap<String, TypeRef>,
) -> ir::Type {
    lower_type_ref_inner(reference, type_aliases, &mut HashSet::new())
}

fn canonicalize_nested_ir_type(
    ty: &mut ir::Type,
    owner: &str,
    names: &HashSet<String>,
    aliases: &HashMap<String, TypeRef>,
) {
    let alias_names = aliases.keys().cloned().collect::<HashSet<_>>();
    canonicalize_nested_ir_type_with_aliases(
        ty,
        owner,
        names,
        aliases,
        &alias_names,
        &mut HashSet::new(),
    );
}

fn canonicalize_nested_ir_type_with_aliases(
    ty: &mut ir::Type,
    owner: &str,
    names: &HashSet<String>,
    aliases: &HashMap<String, TypeRef>,
    alias_names: &HashSet<String>,
    visiting: &mut HashSet<String>,
) {
    match ty {
        ir::Type::Named { name, args } => {
            for arg in args.iter_mut() {
                canonicalize_nested_ir_type_with_aliases(
                    arg,
                    owner,
                    names,
                    aliases,
                    alias_names,
                    visiting,
                );
            }
            if args.is_empty()
                && let Some(alias_name) = lexical_nested_ir_name(owner, name, alias_names)
                && visiting.insert(alias_name.clone())
            {
                let target = aliases.get(&alias_name).expect("known nested type alias");
                let mut lowered = lower_type_ref_with_aliases(target, aliases);
                let alias_owner = alias_name
                    .rsplit_once('.')
                    .map(|(parent, _)| parent)
                    .unwrap_or(owner);
                canonicalize_nested_ir_type_with_aliases(
                    &mut lowered,
                    alias_owner,
                    names,
                    aliases,
                    alias_names,
                    visiting,
                );
                visiting.remove(&alias_name);
                *ty = lowered;
                return;
            }
            if let Some(canonical) = lexical_nested_ir_name(owner, name, names) {
                *name = canonical;
            }
        }
        ir::Type::Union(members) | ir::Type::Tuple(members) => {
            for member in members {
                canonicalize_nested_ir_type_with_aliases(
                    member,
                    owner,
                    names,
                    aliases,
                    alias_names,
                    visiting,
                );
            }
        }
        ir::Type::Record(fields) => {
            for field in fields {
                canonicalize_nested_ir_type_with_aliases(
                    &mut field.ty,
                    owner,
                    names,
                    aliases,
                    alias_names,
                    visiting,
                );
            }
        }
        ir::Type::Function { params, ret } => {
            for param in params {
                canonicalize_nested_ir_type_with_aliases(
                    param,
                    owner,
                    names,
                    aliases,
                    alias_names,
                    visiting,
                );
            }
            canonicalize_nested_ir_type_with_aliases(
                ret,
                owner,
                names,
                aliases,
                alias_names,
                visiting,
            );
        }
        ir::Type::Unknown
        | ir::Type::Never
        | ir::Type::Unit
        | ir::Type::Bool
        | ir::Type::Int
        | ir::Type::Float
        | ir::Type::Str
        | ir::Type::TypeParam(_) => {}
    }
}

fn lexical_nested_ir_name(owner: &str, name: &str, names: &HashSet<String>) -> Option<String> {
    if name.contains('.') {
        return names.contains(name).then(|| name.to_string());
    }
    let mut scope = owner;
    loop {
        let candidate = format!("{scope}.{name}");
        if names.contains(&candidate) {
            return Some(candidate);
        }
        let Some((parent, _)) = scope.rsplit_once('.') else {
            return None;
        };
        scope = parent;
    }
}

fn lower_type_ref_inner(
    reference: &TypeRef,
    type_aliases: &HashMap<String, TypeRef>,
    visiting: &mut HashSet<String>,
) -> ir::Type {
    match reference {
        TypeRef::Wildcard { .. } => ir::Type::Unknown,
        TypeRef::Named { name, args, .. } if name == "Never" && args.is_empty() => ir::Type::Never,
        TypeRef::Named { name, args, .. } if args.is_empty() && type_aliases.contains_key(name) => {
            if !visiting.insert(name.clone()) {
                return ir::Type::Unknown;
            }
            let lowered = lower_type_ref_inner(
                type_aliases.get(name).expect("known type alias"),
                type_aliases,
                visiting,
            );
            visiting.remove(name);
            lowered
        }
        TypeRef::Named { name, args, .. } => ir::Type::Named {
            name: name.clone(),
            args: args
                .iter()
                .map(|arg| lower_type_ref_inner(arg, type_aliases, visiting))
                .collect(),
        },
        TypeRef::Union { members, .. } => ir::Type::Union(
            members
                .iter()
                .map(|member| lower_type_ref_inner(member, type_aliases, visiting))
                .collect(),
        ),
        TypeRef::Tuple { fields, .. } => ir::Type::Tuple(
            fields
                .iter()
                .map(|field| lower_type_ref_inner(&field.ty, type_aliases, visiting))
                .collect(),
        ),
        TypeRef::Record { fields, .. } => ir::Type::Record(
            fields
                .iter()
                .map(|field| ir::NamedType {
                    name: field.name.clone(),
                    ty: lower_type_ref_inner(&field.ty, type_aliases, visiting),
                })
                .collect(),
        ),
        TypeRef::Function { params, ret, .. } => ir::Type::Function {
            params: params
                .iter()
                .map(|param| lower_type_ref_inner(param, type_aliases, visiting))
                .collect(),
            ret: Box::new(lower_type_ref_inner(ret, type_aliases, visiting)),
        },
    }
}

fn lower_runtime_type_ref_has_arguments(reference: &TypeRef) -> bool {
    match reference {
        TypeRef::Named { args, .. } => !args.is_empty(),
        TypeRef::Tuple { fields, .. } => fields
            .iter()
            .any(|field| lower_runtime_type_ref_has_arguments(&field.ty)),
        TypeRef::Record { fields, .. } => fields
            .iter()
            .any(|field| lower_runtime_type_ref_has_arguments(&field.ty)),
        TypeRef::Function { params, ret, .. } => {
            params.iter().any(lower_runtime_type_ref_has_arguments)
                || lower_runtime_type_ref_has_arguments(ret)
        }
        TypeRef::Union { members, .. } => members.iter().any(lower_runtime_type_ref_has_arguments),
        TypeRef::Wildcard { .. } => false,
    }
}

fn lazy_storage_type(value_ty: ir::Type) -> ir::Type {
    ir::Type::Function {
        params: Vec::new(),
        ret: Box::new(value_ty),
    }
}

fn lazy_value_type(storage_ty: &ir::Type) -> Option<ir::Type> {
    match storage_ty {
        ir::Type::Function { params, ret } if params.is_empty() => Some((**ret).clone()),
        _ => None,
    }
}

fn is_builtin_extension_target(name: &str) -> bool {
    matches!(name, "Bool" | "Float" | "Int" | "Rune" | "Str")
}

fn builtin_extension_receiver_name(ty: &ir::Type) -> Option<&'static str> {
    match ty {
        ir::Type::Bool => Some("Bool"),
        ir::Type::Int => Some("Int"),
        ir::Type::Float => Some("Float"),
        ir::Type::Str => Some("Str"),
        _ => None,
    }
}

fn lower_type_ref_with_type_params(
    reference: &TypeRef,
    type_params: &[String],
    type_aliases: &HashMap<String, TypeRef>,
) -> ir::Type {
    lower_type_ref_with_type_params_inner(reference, type_params, type_aliases, &mut HashSet::new())
}

fn lower_type_ref_with_type_params_inner(
    reference: &TypeRef,
    type_params: &[String],
    type_aliases: &HashMap<String, TypeRef>,
    visiting: &mut HashSet<String>,
) -> ir::Type {
    match reference {
        TypeRef::Named { name, args, .. }
            if args.is_empty() && type_params.iter().any(|param| param == name) =>
        {
            ir::Type::TypeParam(name.clone())
        }
        TypeRef::Named { name, args, .. } if name == "Never" && args.is_empty() => ir::Type::Never,
        TypeRef::Named { name, args, .. } if args.is_empty() && type_aliases.contains_key(name) => {
            if !visiting.insert(name.clone()) {
                return ir::Type::Unknown;
            }
            let lowered = lower_type_ref_with_type_params_inner(
                type_aliases.get(name).expect("known type alias"),
                type_params,
                type_aliases,
                visiting,
            );
            visiting.remove(name);
            lowered
        }
        TypeRef::Named { name, args, .. } => ir::Type::Named {
            name: name.clone(),
            args: args
                .iter()
                .map(|arg| {
                    lower_type_ref_with_type_params_inner(arg, type_params, type_aliases, visiting)
                })
                .collect(),
        },
        TypeRef::Union { members, .. } => ir::Type::Union(
            members
                .iter()
                .map(|member| {
                    lower_type_ref_with_type_params_inner(
                        member,
                        type_params,
                        type_aliases,
                        visiting,
                    )
                })
                .collect(),
        ),
        TypeRef::Wildcard { .. } => ir::Type::Unknown,
        TypeRef::Tuple { fields, .. } => ir::Type::Tuple(
            fields
                .iter()
                .map(|field| {
                    lower_type_ref_with_type_params_inner(
                        &field.ty,
                        type_params,
                        type_aliases,
                        visiting,
                    )
                })
                .collect(),
        ),
        TypeRef::Record { fields, .. } => ir::Type::Record(
            fields
                .iter()
                .map(|field| ir::NamedType {
                    name: field.name.clone(),
                    ty: lower_type_ref_with_type_params_inner(
                        &field.ty,
                        type_params,
                        type_aliases,
                        visiting,
                    ),
                })
                .collect(),
        ),
        TypeRef::Function { params, ret, .. } => ir::Type::Function {
            params: params
                .iter()
                .map(|param| {
                    lower_type_ref_with_type_params_inner(
                        param,
                        type_params,
                        type_aliases,
                        visiting,
                    )
                })
                .collect(),
            ret: Box::new(lower_type_ref_with_type_params_inner(
                ret,
                type_params,
                type_aliases,
                visiting,
            )),
        },
    }
}

fn lower_generic_conditions(
    params: &[ast::TypeParam],
    conditions: &[ast::GenericCondition],
    type_params: &[String],
    type_aliases: &HashMap<String, TypeRef>,
) -> Vec<ir::GenericCondition> {
    let mut lowered = Vec::new();
    for param in params {
        for bound in &param.bounds {
            lowered.push(ir::GenericCondition::Bound {
                subject: ir::Type::TypeParam(param.name.clone()),
                bound: lower_type_ref_with_type_params(bound, type_params, type_aliases),
            });
        }
    }
    lowered.extend(conditions.iter().map(|condition| match condition {
        ast::GenericCondition::Bound { subject, bound, .. } => ir::GenericCondition::Bound {
            subject: lower_type_ref_with_type_params(subject, type_params, type_aliases),
            bound: lower_type_ref_with_type_params(bound, type_params, type_aliases),
        },
        ast::GenericCondition::Equal { left, right, .. } => ir::GenericCondition::Equal {
            left: lower_type_ref_with_type_params(left, type_params, type_aliases),
            right: lower_type_ref_with_type_params(right, type_params, type_aliases),
        },
    }));
    lowered
}

fn ir_exact_runtime_type(represented: ir::Type) -> ir::Type {
    ir::Type::Named {
        name: "Type".to_string(),
        args: vec![match represented {
            ir::Type::Unknown | ir::Type::Never => ir::Type::Unknown,
            other => other,
        }],
    }
}

fn ir_value_runtime_type(represented: ir::Type) -> ir::Type {
    ir::Type::Named {
        name: "Type".to_string(),
        args: vec![match represented {
            ir::Type::Unknown | ir::Type::Never => ir::Type::Unknown,
            ir::Type::Named { name, args } if name == "Any" && args.is_empty() => ir::Type::Unknown,
            other => other,
        }],
    }
}

fn lower_annotations(annotations: &[ast::Annotation]) -> Vec<ir::Annotation> {
    annotations.iter().filter_map(lower_annotation).collect()
}

fn lower_annotation(annotation: &ast::Annotation) -> Option<ir::Annotation> {
    let (name, fields) = match &annotation.value {
        ast::Expr::Call {
            callee,
            args,
            uses_brace_syntax,
            ..
        } => {
            let name = annotation_expr_name(callee)?;
            let fields = if *uses_brace_syntax && args.len() == 1 && args[0].name.is_none() {
                match &args[0].value {
                    ast::Expr::RecordLiteral { fields, .. } => fields
                        .iter()
                        .filter_map(|field| {
                            Some(ir::AnnotationField {
                                name: field.name.clone()?,
                                value: lower_annotation_value(&field.value),
                            })
                        })
                        .collect(),
                    _ => Vec::new(),
                }
            } else {
                args.iter()
                    .filter_map(|arg| {
                        Some(ir::AnnotationField {
                            name: arg.name.clone()?,
                            value: lower_annotation_value(&arg.value),
                        })
                    })
                    .collect()
            };
            (name, fields)
        }
        other => (annotation_expr_name(other)?, Vec::new()),
    };
    Some(ir::Annotation { name, fields })
}

fn annotation_expr_name(expr: &ast::Expr) -> Option<String> {
    match expr {
        ast::Expr::Identifier { name, .. } => Some(name.clone()),
        ast::Expr::Member { receiver, name, .. } => {
            let mut path = annotation_expr_name(receiver)?;
            path.push('.');
            path.push_str(name);
            Some(path)
        }
        ast::Expr::Group { inner, .. } => annotation_expr_name(inner),
        _ => None,
    }
}

fn lower_annotation_value(expr: &ast::Expr) -> ir::AnnotationValue {
    match expr {
        ast::Expr::Group { inner, .. } => lower_annotation_value(inner),
        ast::Expr::Bool { value, .. } => ir::AnnotationValue::Bool(*value),
        ast::Expr::Integer { raw, .. } => {
            ir::AnnotationValue::Int(raw.parse::<i64>().unwrap_or_default())
        }
        ast::Expr::Float { raw, .. } => {
            ir::AnnotationValue::Float(raw.parse::<f64>().unwrap_or_default())
        }
        ast::Expr::String { raw, .. } => ir::AnnotationValue::String(annotation_string_value(raw)),
        ast::Expr::ListLiteral { items, .. } => {
            ir::AnnotationValue::List(items.iter().map(lower_annotation_value).collect())
        }
        ast::Expr::RecordLiteral { fields, .. } => ir::AnnotationValue::Record(
            fields
                .iter()
                .filter_map(|field| {
                    Some(ir::AnnotationField {
                        name: field.name.clone()?,
                        value: lower_annotation_value(&field.value),
                    })
                })
                .collect(),
        ),
        ast::Expr::Member { .. } => annotation_expr_name(expr)
            .map(|name| {
                ir::AnnotationValue::EnumCase(name.split('.').map(str::to_string).collect())
            })
            .unwrap_or_else(|| ir::AnnotationValue::Unresolved(String::new())),
        ast::Expr::Binary {
            left,
            op: AstBinaryOp::Add,
            right,
            ..
        } => match (lower_annotation_value(left), lower_annotation_value(right)) {
            (ir::AnnotationValue::String(left), ir::AnnotationValue::String(right)) => {
                ir::AnnotationValue::String(format!("{left}{right}"))
            }
            (ir::AnnotationValue::Int(left), ir::AnnotationValue::Int(right)) => {
                ir::AnnotationValue::Int(left + right)
            }
            (left, right) => ir::AnnotationValue::Unresolved(format!("{left:?} + {right:?}")),
        },
        other => ir::AnnotationValue::Unresolved(format!("{other:?}")),
    }
}

fn annotation_string_value(raw: &str) -> String {
    let raw = raw.strip_prefix("raw").unwrap_or(raw);
    if let Some(body) = raw
        .strip_prefix("\"\"\"")
        .and_then(|v| v.strip_suffix("\"\"\""))
    {
        return body.to_string();
    }
    raw.strip_prefix('"')
        .and_then(|v| v.strip_suffix('"'))
        .unwrap_or(raw)
        .to_string()
}

fn lower_lambda_param_type(
    param: &core::LambdaParam,
    expected: Option<&ir::Type>,
    type_aliases: &HashMap<String, TypeRef>,
) -> ir::Type {
    if let Some(ty) = &param.ty {
        return lower_type_ref_with_aliases(ty, type_aliases);
    }
    let Some(destructure) = &param.destructure else {
        return expected.cloned().unwrap_or(ir::Type::Unknown);
    };
    if let Some(expected) = expected {
        return expected.clone();
    }
    match destructure.kind {
        DestructureKind::Tuple => ir::Type::Tuple(
            destructure
                .bindings
                .iter()
                .map(|binding| {
                    binding
                        .ty
                        .as_ref()
                        .map(|ty| lower_type_ref_with_aliases(ty, type_aliases))
                        .unwrap_or(ir::Type::Unknown)
                })
                .collect(),
        ),
        DestructureKind::Record => ir::Type::Record(
            destructure
                .bindings
                .iter()
                .map(|binding| ir::NamedType {
                    name: binding
                        .field_name
                        .clone()
                        .unwrap_or_else(|| binding.name.clone()),
                    ty: binding
                        .ty
                        .as_ref()
                        .map(|ty| lower_type_ref_with_aliases(ty, type_aliases))
                        .unwrap_or(ir::Type::Unknown),
                })
                .collect(),
        ),
    }
}

fn lower_lambda_param_name(param: &core::LambdaParam, index: usize) -> String {
    if param.name == "_" {
        format!("$ignored{index}")
    } else {
        param.name.clone()
    }
}

fn upsert_ir_record_field(fields: &mut Vec<ir::NamedType>, field: ir::NamedType) {
    if let Some(existing) = fields
        .iter_mut()
        .find(|existing| existing.name == field.name)
    {
        *existing = field;
    } else {
        fields.push(field);
    }
}

fn join_ir_types(left: ir::Type, right: ir::Type) -> ir::Type {
    match (&left, &right) {
        (ir::Type::Unknown, _) => right,
        (_, ir::Type::Unknown) => left,
        _ if left == right => left,
        _ => ir::Type::Unknown,
    }
}

fn unwrap_lifted_ir_type(ty: &ir::Type) -> Option<(LiftedIrFamily, ir::Type)> {
    match ty {
        ir::Type::Named { name, args } if name == "Option" && args.len() == 1 => {
            Some((LiftedIrFamily::Option, args[0].clone()))
        }
        ir::Type::Named { name, args } if name == "Result" && args.len() == 2 => Some((
            LiftedIrFamily::Result {
                error: args[1].clone(),
            },
            args[0].clone(),
        )),
        ir::Type::Named { name, args } if name == "Either" && args.len() == 2 => Some((
            LiftedIrFamily::Either {
                left: args[0].clone(),
            },
            args[1].clone(),
        )),
        ir::Type::Unknown => Some((LiftedIrFamily::Option, ir::Type::Unknown)),
        _ => None,
    }
}

fn known_lifted_ir_type(ty: &ir::Type) -> Option<(LiftedIrFamily, ir::Type)> {
    match ty {
        ir::Type::Unknown => None,
        _ => unwrap_lifted_ir_type(ty),
    }
}

fn wrap_lifted_ir_type(family: &LiftedIrFamily, inner: ir::Type) -> ir::Type {
    match family {
        LiftedIrFamily::Option => ir::Type::option(inner),
        LiftedIrFamily::Result { error } => ir::Type::Named {
            name: "Result".to_string(),
            args: vec![inner, error.clone()],
        },
        LiftedIrFamily::Either { left } => ir::Type::Named {
            name: "Either".to_string(),
            args: vec![left.clone(), inner],
        },
    }
}

fn index_result_ir_type(ty: &ir::Type) -> ir::Type {
    match ty {
        ir::Type::Named { name, args } if name == "LinkedList" && args.len() == 1 => {
            ir::Type::option(args[0].clone())
        }
        ir::Type::Named { name, args }
            if (name == "Vector" || name == "Array") && args.len() == 1 =>
        {
            args[0].clone()
        }
        ir::Type::Named { name, args } if name == "Map" && args.len() == 2 => {
            ir::Type::option(args[1].clone())
        }
        ir::Type::Tuple(items) => items
            .iter()
            .cloned()
            .reduce(join_ir_types)
            .unwrap_or(ir::Type::Unknown),
        ir::Type::Unknown => ir::Type::Unknown,
        _ => ir::Type::Unknown,
    }
}

fn static_tuple_index(expr: &Expr) -> Option<usize> {
    match expr {
        Expr::Integer { raw, .. } => raw.parse().ok(),
        _ => None,
    }
}

fn known_iterable_ir_item_type(ty: &ir::Type) -> Option<ir::Type> {
    match ty {
        ir::Type::Named { name, args }
            if (name == "Vector"
                || name == "Iterable"
                || name == "Iterator"
                || name == "Array"
                || name == "LinkedList"
                || name == "Option"
                || name == "Set")
                && args.len() == 1 =>
        {
            Some(args[0].clone())
        }
        ir::Type::Named { name, args } if name == "Map" && args.len() == 2 => {
            Some(ir::Type::Tuple(vec![args[0].clone(), args[1].clone()]))
        }
        ir::Type::Named { name, args } if name == "IntRange" && args.is_empty() => {
            Some(ir::Type::Int)
        }
        _ => None,
    }
}

fn builtin_member_expected_arg_specs(
    receiver: &ir::Type,
    name: &str,
    expected: Option<&ir::Type>,
) -> Option<Vec<Option<ExpectedArgSpec>>> {
    let ir::Type::Named {
        name: type_name,
        args,
    } = receiver
    else {
        return None;
    };
    let item = args.first().cloned().unwrap_or(ir::Type::Unknown);
    let spec = |ty: ir::Type, lazy: bool| {
        Some(ExpectedArgSpec {
            name: None,
            ty,
            lazy,
            variadic: false,
            default: None,
        })
    };
    match (type_name.as_str(), name) {
        ("Database", "transactionally") => {
            let value_ty = transactionally_result_value_type(expected).unwrap_or(ir::Type::Unknown);
            Some(vec![spec(transactional_work_type(value_ty), false)])
        }
        ("Option", "map") => {
            let mapped = match expected {
                Some(ir::Type::Named { name, args }) if name == "Option" && args.len() == 1 => {
                    args[0].clone()
                }
                _ => ir::Type::Unknown,
            };
            Some(vec![spec(
                ir::Type::Function {
                    params: vec![item.clone()],
                    ret: Box::new(mapped),
                },
                false,
            )])
        }
        ("Option", "flatMap") => {
            let mapped = match expected {
                Some(ir::Type::Named { name, args }) if name == "Option" && args.len() == 1 => {
                    ir::Type::option(args[0].clone())
                }
                _ => ir::Type::option(ir::Type::Unknown),
            };
            Some(vec![spec(
                ir::Type::Function {
                    params: vec![item.clone()],
                    ret: Box::new(mapped),
                },
                false,
            )])
        }
        ("Option", "orElse") => Some(vec![spec(receiver.clone(), true)]),
        ("Option", "toResult") => {
            let error = match expected {
                Some(ir::Type::Named { name, args }) if name == "Result" && args.len() == 2 => {
                    args[1].clone()
                }
                _ => ir::Type::Unknown,
            };
            Some(vec![spec(error, true)])
        }
        ("Option", "toEither") => {
            let left = match expected {
                Some(ir::Type::Named { name, args }) if name == "Either" && args.len() == 2 => {
                    args[0].clone()
                }
                _ => ir::Type::Unknown,
            };
            Some(vec![spec(left, true)])
        }
        ("Result", "map") => {
            let mapped = match expected {
                Some(ir::Type::Named { name, args }) if name == "Result" && args.len() == 2 => {
                    args[0].clone()
                }
                _ => ir::Type::Unknown,
            };
            Some(vec![spec(
                ir::Type::Function {
                    params: vec![item.clone()],
                    ret: Box::new(mapped),
                },
                false,
            )])
        }
        ("Result", "flatMap") => {
            let error = args.get(1).cloned().unwrap_or(ir::Type::Unknown);
            let mapped = match expected {
                Some(ir::Type::Named { name, args }) if name == "Result" && args.len() == 2 => {
                    ir::Type::Named {
                        name: "Result".to_string(),
                        args: vec![args[0].clone(), error.clone()],
                    }
                }
                _ => ir::Type::Named {
                    name: "Result".to_string(),
                    args: vec![ir::Type::Unknown, error],
                },
            };
            Some(vec![spec(
                ir::Type::Function {
                    params: vec![item.clone()],
                    ret: Box::new(mapped),
                },
                false,
            )])
        }
        ("Result", "orElse") => Some(vec![spec(receiver.clone(), true)]),
        ("Result", "mapError") => {
            let error = args.get(1).cloned().unwrap_or(ir::Type::Unknown);
            let mapped = match expected {
                Some(ir::Type::Named { name, args }) if name == "Result" && args.len() == 2 => {
                    args[1].clone()
                }
                _ => ir::Type::Unknown,
            };
            Some(vec![spec(
                ir::Type::Function {
                    params: vec![error],
                    ret: Box::new(mapped),
                },
                false,
            )])
        }
        ("Either", "map") => {
            let right = args.get(1).cloned().unwrap_or(ir::Type::Unknown);
            let mapped = match expected {
                Some(ir::Type::Named { name, args }) if name == "Either" && args.len() == 2 => {
                    args[1].clone()
                }
                _ => ir::Type::Unknown,
            };
            Some(vec![spec(
                ir::Type::Function {
                    params: vec![right],
                    ret: Box::new(mapped),
                },
                false,
            )])
        }
        ("Either", "flatMap") => {
            let left = args.first().cloned().unwrap_or(ir::Type::Unknown);
            let right = args.get(1).cloned().unwrap_or(ir::Type::Unknown);
            let mapped = match expected {
                Some(ir::Type::Named { name, args }) if name == "Either" && args.len() == 2 => {
                    ir::Type::Named {
                        name: "Either".to_string(),
                        args: vec![left.clone(), args[1].clone()],
                    }
                }
                _ => ir::Type::Named {
                    name: "Either".to_string(),
                    args: vec![left, ir::Type::Unknown],
                },
            };
            Some(vec![spec(
                ir::Type::Function {
                    params: vec![right],
                    ret: Box::new(mapped),
                },
                false,
            )])
        }
        ("Either", "orElse") => Some(vec![spec(receiver.clone(), true)]),
        ("Either", "merge") => Some(Vec::new()),
        ("Either", "mapLeft") => {
            let left = args.first().cloned().unwrap_or(ir::Type::Unknown);
            let mapped = match expected {
                Some(ir::Type::Named { name, args }) if name == "Either" && args.len() == 2 => {
                    args[0].clone()
                }
                _ => ir::Type::Unknown,
            };
            Some(vec![spec(
                ir::Type::Function {
                    params: vec![left],
                    ret: Box::new(mapped),
                },
                false,
            )])
        }
        _ => None,
    }
}

fn transactionally_result_value_type(expected: Option<&ir::Type>) -> Option<ir::Type> {
    match expected {
        Some(ir::Type::Named { name, args }) if name == "Result" && args.len() == 2 => {
            Some(args[0].clone())
        }
        _ => None,
    }
}

fn transactional_work_type(value_ty: ir::Type) -> ir::Type {
    ir::Type::Function {
        params: vec![ir::Type::Named {
            name: "Transaction".to_string(),
            args: Vec::new(),
        }],
        ret: Box::new(result_db_error_type(value_ty)),
    }
}

fn result_db_error_type(value_ty: ir::Type) -> ir::Type {
    ir::Type::Named {
        name: "Result".to_string(),
        args: vec![
            value_ty,
            ir::Type::Named {
                name: "DbError".to_string(),
                args: Vec::new(),
            },
        ],
    }
}

fn builtin_member_type(receiver: &ir::Type, name: &str) -> Option<ir::Type> {
    if let Some(ty) = universal_member_type(name) {
        return Some(ty);
    }
    if let Some(ty) = builtin_getter_type(receiver, name) {
        return Some(ty);
    }

    let ir::Type::Named {
        name: type_name,
        args,
    } = receiver
    else {
        return None;
    };
    let item = args.first().cloned().unwrap_or(ir::Type::Unknown);
    match (type_name.as_str(), name) {
        ("Database", "transactionally") => Some(ir::Type::Function {
            params: vec![transactional_work_type(ir::Type::Unknown)],
            ret: Box::new(result_db_error_type(ir::Type::Unknown)),
        }),
        ("Option", "map") => Some(ir::Type::Function {
            params: vec![ir::Type::Function {
                params: vec![item.clone()],
                ret: Box::new(ir::Type::Unknown),
            }],
            ret: Box::new(ir::Type::option(ir::Type::Unknown)),
        }),
        ("Option", "flatMap") => Some(ir::Type::Function {
            params: vec![ir::Type::Function {
                params: vec![item.clone()],
                ret: Box::new(ir::Type::option(ir::Type::Unknown)),
            }],
            ret: Box::new(ir::Type::option(ir::Type::Unknown)),
        }),
        ("Option", "orElse") => Some(ir::Type::Function {
            params: vec![receiver.clone()],
            ret: Box::new(receiver.clone()),
        }),
        ("Option", "toResult") => Some(ir::Type::Function {
            params: vec![ir::Type::Unknown],
            ret: Box::new(ir::Type::Named {
                name: "Result".to_string(),
                args: vec![item.clone(), ir::Type::Unknown],
            }),
        }),
        ("Option", "toEither") => Some(ir::Type::Function {
            params: vec![ir::Type::Unknown],
            ret: Box::new(ir::Type::Named {
                name: "Either".to_string(),
                args: vec![ir::Type::Unknown, item.clone()],
            }),
        }),
        ("Option", "isDefined") => Some(ir::Type::Function {
            params: Vec::new(),
            ret: Box::new(ir::Type::Bool),
        }),
        ("Result", "map") => {
            let error = args.get(1).cloned().unwrap_or(ir::Type::Unknown);
            Some(ir::Type::Function {
                params: vec![ir::Type::Function {
                    params: vec![item.clone()],
                    ret: Box::new(ir::Type::Unknown),
                }],
                ret: Box::new(ir::Type::Named {
                    name: "Result".to_string(),
                    args: vec![ir::Type::Unknown, error],
                }),
            })
        }
        ("Result", "flatMap") => {
            let error = args.get(1).cloned().unwrap_or(ir::Type::Unknown);
            Some(ir::Type::Function {
                params: vec![ir::Type::Function {
                    params: vec![item.clone()],
                    ret: Box::new(ir::Type::Named {
                        name: "Result".to_string(),
                        args: vec![ir::Type::Unknown, error.clone()],
                    }),
                }],
                ret: Box::new(ir::Type::Named {
                    name: "Result".to_string(),
                    args: vec![ir::Type::Unknown, error],
                }),
            })
        }
        ("Result", "orElse") => Some(ir::Type::Function {
            params: vec![receiver.clone()],
            ret: Box::new(receiver.clone()),
        }),
        ("Result", "mapError") => {
            let error = args.get(1).cloned().unwrap_or(ir::Type::Unknown);
            Some(ir::Type::Function {
                params: vec![ir::Type::Function {
                    params: vec![error],
                    ret: Box::new(ir::Type::Unknown),
                }],
                ret: Box::new(ir::Type::Named {
                    name: "Result".to_string(),
                    args: vec![item.clone(), ir::Type::Unknown],
                }),
            })
        }
        ("Either", "map") => {
            let left = args.first().cloned().unwrap_or(ir::Type::Unknown);
            let right = args.get(1).cloned().unwrap_or(ir::Type::Unknown);
            Some(ir::Type::Function {
                params: vec![ir::Type::Function {
                    params: vec![right],
                    ret: Box::new(ir::Type::Unknown),
                }],
                ret: Box::new(ir::Type::Named {
                    name: "Either".to_string(),
                    args: vec![left, ir::Type::Unknown],
                }),
            })
        }
        ("Either", "flatMap") => {
            let left = args.first().cloned().unwrap_or(ir::Type::Unknown);
            let right = args.get(1).cloned().unwrap_or(ir::Type::Unknown);
            Some(ir::Type::Function {
                params: vec![ir::Type::Function {
                    params: vec![right],
                    ret: Box::new(ir::Type::Named {
                        name: "Either".to_string(),
                        args: vec![left.clone(), ir::Type::Unknown],
                    }),
                }],
                ret: Box::new(ir::Type::Named {
                    name: "Either".to_string(),
                    args: vec![left, ir::Type::Unknown],
                }),
            })
        }
        ("Either", "orElse") => Some(ir::Type::Function {
            params: vec![receiver.clone()],
            ret: Box::new(receiver.clone()),
        }),
        ("Either", "merge") => Some(ir::Type::Function {
            params: Vec::new(),
            ret: Box::new(item.clone()),
        }),
        ("Either", "mapLeft") => {
            let left = args.first().cloned().unwrap_or(ir::Type::Unknown);
            let right = args.get(1).cloned().unwrap_or(ir::Type::Unknown);
            Some(ir::Type::Function {
                params: vec![ir::Type::Function {
                    params: vec![left],
                    ret: Box::new(ir::Type::Unknown),
                }],
                ret: Box::new(ir::Type::Named {
                    name: "Either".to_string(),
                    args: vec![ir::Type::Unknown, right],
                }),
            })
        }
        ("Type", "name" | "qualifiedName") => Some(ir::Type::Function {
            params: Vec::new(),
            ret: Box::new(ir::Type::option(ir::Type::Str)),
        }),
        ("Type", "kind") => Some(ir::Type::Function {
            params: Vec::new(),
            ret: Box::new(ir::Type::named("TypeKind")),
        }),
        ("Type", "asClass") => Some(ir::Type::Function {
            params: Vec::new(),
            ret: Box::new(ir::Type::option(ir::Type::named("ClassType"))),
        }),
        ("Type", "asShape") => Some(ir::Type::Function {
            params: Vec::new(),
            ret: Box::new(ir::Type::option(ir::Type::named("ShapeType"))),
        }),
        ("Type", "asEnum") => Some(ir::Type::Function {
            params: Vec::new(),
            ret: Box::new(ir::Type::option(ir::Type::named("EnumType"))),
        }),
        ("Type", "asInterface") => Some(ir::Type::Function {
            params: Vec::new(),
            ret: Box::new(ir::Type::option(ir::Type::named("InterfaceType"))),
        }),
        ("Type", "asObject") => Some(ir::Type::Function {
            params: Vec::new(),
            ret: Box::new(ir::Type::option(ir::Type::named("ObjectType"))),
        }),
        ("Type", "asAnnotation") => Some(ir::Type::Function {
            params: Vec::new(),
            ret: Box::new(ir::Type::option(ir::Type::named("AnnotationType"))),
        }),
        (
            "ClassType" | "ShapeType" | "EnumType" | "InterfaceType" | "ObjectType"
            | "AnnotationType",
            "name" | "qualifiedName",
        ) => Some(ir::Type::Function {
            params: Vec::new(),
            ret: Box::new(ir::Type::option(ir::Type::Str)),
        }),
        (
            "ClassType" | "ShapeType" | "EnumType" | "InterfaceType" | "ObjectType"
            | "AnnotationType",
            "kind",
        ) => Some(ir::Type::Function {
            params: Vec::new(),
            ret: Box::new(ir::Type::named("TypeKind")),
        }),
        (
            "Type" | "ClassType" | "ShapeType" | "EnumType" | "InterfaceType" | "ObjectType"
            | "AnnotationType" | "Field" | "Method" | "EnumCase",
            "annotation",
        ) => Some(ir::Type::Function {
            params: Vec::new(),
            ret: Box::new(ir::Type::option(ir::Type::named("AnnotationValue"))),
        }),
        (
            "Type" | "ClassType" | "ShapeType" | "EnumType" | "InterfaceType" | "ObjectType"
            | "AnnotationType" | "Field" | "Method" | "EnumCase",
            "hasAnnotation",
        ) => Some(ir::Type::Function {
            params: Vec::new(),
            ret: Box::new(ir::Type::Bool),
        }),
        ("AnnotationValue", "name") => Some(ir::Type::Function {
            params: Vec::new(),
            ret: Box::new(ir::Type::Str),
        }),
        ("AnnotationValue", "field") => Some(ir::Type::Function {
            params: vec![ir::Type::Str],
            ret: Box::new(ir::Type::option(ir::Type::named("Any"))),
        }),
        ("AnnotationValue", "str") => Some(ir::Type::Function {
            params: vec![ir::Type::Str],
            ret: Box::new(ir::Type::option(ir::Type::Str)),
        }),
        ("ClassType" | "ShapeType" | "ObjectType" | "AnnotationType", "fields") => {
            Some(ir::Type::Function {
                params: Vec::new(),
                ret: Box::new(ir::Type::list(ir::Type::named("Field"))),
            })
        }
        ("ClassType" | "ObjectType", "field") => Some(ir::Type::Function {
            params: vec![ir::Type::Str],
            ret: Box::new(ir::Type::option(ir::Type::named("Field"))),
        }),
        ("ClassType" | "ShapeType" | "EnumType" | "InterfaceType" | "ObjectType", "methods") => {
            Some(ir::Type::Function {
                params: Vec::new(),
                ret: Box::new(ir::Type::list(ir::Type::named("Method"))),
            })
        }
        ("ClassType" | "ObjectType", "method") => Some(ir::Type::Function {
            params: vec![ir::Type::Str],
            ret: Box::new(ir::Type::option(ir::Type::named("Method"))),
        }),
        ("ClassType" | "ShapeType", "construct") => Some(ir::Type::Function {
            params: vec![ir::Type::list(ir::Type::named("Any"))],
            ret: Box::new(ir::Type::Named {
                name: "Result".to_string(),
                args: vec![ir::Type::named("Any"), ir::Type::named("ReflectionError")],
            }),
        }),
        ("EnumType", "cases") => Some(ir::Type::Function {
            params: Vec::new(),
            ret: Box::new(ir::Type::list(ir::Type::named("EnumCase"))),
        }),
        ("EnumType", "case") => Some(ir::Type::Function {
            params: vec![ir::Type::Str],
            ret: Box::new(ir::Type::option(ir::Type::named("EnumCase"))),
        }),
        ("Field", "name") => Some(ir::Type::Function {
            params: Vec::new(),
            ret: Box::new(ir::Type::Str),
        }),
        ("Field", "fieldType") => Some(ir::Type::Function {
            params: Vec::new(),
            ret: Box::new(ir_exact_runtime_type(ir::Type::Unknown)),
        }),
        ("Field", "isPrivate") => Some(ir::Type::Function {
            params: Vec::new(),
            ret: Box::new(ir::Type::Bool),
        }),
        ("Field", "get") => Some(ir::Type::Function {
            params: vec![ir::Type::named("Any")],
            ret: Box::new(ir::Type::Named {
                name: "Result".to_string(),
                args: vec![ir::Type::named("Any"), ir::Type::named("ReflectionError")],
            }),
        }),
        ("Method", "name") => Some(ir::Type::Function {
            params: Vec::new(),
            ret: Box::new(ir::Type::Str),
        }),
        ("Method", "params") => Some(ir::Type::Function {
            params: Vec::new(),
            ret: Box::new(ir::Type::list(ir::Type::named("Param"))),
        }),
        ("Method", "returnType") => Some(ir::Type::Function {
            params: Vec::new(),
            ret: Box::new(ir_exact_runtime_type(ir::Type::Unknown)),
        }),
        ("Method", "call") => Some(ir::Type::Function {
            params: vec![
                ir::Type::named("Any"),
                ir::Type::list(ir::Type::named("Any")),
            ],
            ret: Box::new(ir::Type::Named {
                name: "Result".to_string(),
                args: vec![ir::Type::named("Any"), ir::Type::named("ReflectionError")],
            }),
        }),
        ("Param", "name") => Some(ir::Type::Function {
            params: Vec::new(),
            ret: Box::new(ir::Type::Str),
        }),
        ("Param", "paramType") => Some(ir::Type::Function {
            params: Vec::new(),
            ret: Box::new(ir_exact_runtime_type(ir::Type::Unknown)),
        }),
        ("EnumCase", "name") => Some(ir::Type::Function {
            params: Vec::new(),
            ret: Box::new(ir::Type::Str),
        }),
        ("EnumCase", "fields") => Some(ir::Type::Function {
            params: Vec::new(),
            ret: Box::new(ir::Type::list(ir::Type::named("Field"))),
        }),
        ("EnumCase", "construct") => Some(ir::Type::Function {
            params: vec![ir::Type::list(ir::Type::named("Any"))],
            ret: Box::new(ir::Type::Named {
                name: "Result".to_string(),
                args: vec![ir::Type::named("Any"), ir::Type::named("ReflectionError")],
            }),
        }),
        (
            "Vector" | "LinkedList" | "Array",
            "head" | "first" | "last" | "removeFirst" | "removeLast",
        ) => Some(ir::Type::Function {
            params: Vec::new(),
            ret: Box::new(ir::Type::option(item)),
        }),
        ("Vector" | "LinkedList" | "Array", "at") => Some(ir::Type::Function {
            params: vec![ir::Type::Int],
            ret: Box::new(ir::Type::option(item)),
        }),
        ("Vector", "slice") => Some(ir::Type::Function {
            params: vec![ir::Type::Int, ir::Type::Int],
            ret: Box::new(receiver.clone()),
        }),
        ("Vector" | "LinkedList" | "Array", "setAt") => Some(ir::Type::Function {
            params: vec![ir::Type::Int, item.clone()],
            ret: Box::new(ir::Type::Named {
                name: "Result".to_string(),
                args: vec![item, ir::Type::named("InvalidIndex")],
            }),
        }),
        ("Vector" | "LinkedList", "insertAt") => Some(ir::Type::Function {
            params: vec![ir::Type::Int, item],
            ret: Box::new(ir::Type::Named {
                name: "Result".to_string(),
                args: vec![ir::Type::Unit, ir::Type::named("InvalidIndex")],
            }),
        }),
        ("Vector" | "LinkedList", "removeAt") => Some(ir::Type::Function {
            params: vec![ir::Type::Int],
            ret: Box::new(ir::Type::Named {
                name: "Result".to_string(),
                args: vec![item, ir::Type::named("InvalidIndex")],
            }),
        }),
        ("Vector" | "LinkedList" | "Array" | "Set", "contains") => Some(ir::Type::Function {
            params: vec![item],
            ret: Box::new(ir::Type::Bool),
        }),
        ("Vector" | "LinkedList" | "Array" | "Set" | "Map" | "Str", "size" | "length") => {
            Some(ir::Type::Function {
                params: Vec::new(),
                ret: Box::new(ir::Type::Int),
            })
        }
        ("Vector" | "LinkedList" | "Array" | "Set" | "Map", "isEmpty" | "nonEmpty") => {
            Some(ir::Type::Function {
                params: Vec::new(),
                ret: Box::new(ir::Type::Bool),
            })
        }
        ("LinkedList", "add" | "append") => Some(ir::Type::Function {
            params: vec![item],
            ret: Box::new(receiver.clone()),
        }),
        ("LinkedList", "fold") => Some(ir::Type::Function {
            params: vec![
                ir::Type::Unknown,
                ir::Type::Function {
                    params: vec![ir::Type::Unknown, item],
                    ret: Box::new(ir::Type::Unknown),
                },
            ],
            ret: Box::new(ir::Type::Unknown),
        }),
        ("Map", "entries") if args.len() == 2 => Some(ir::Type::Function {
            params: Vec::new(),
            ret: Box::new(ir::Type::list(ir::Type::Tuple(vec![
                args[0].clone(),
                args[1].clone(),
            ]))),
        }),
        ("Map", "keys") if args.len() == 2 => Some(ir::Type::Function {
            params: Vec::new(),
            ret: Box::new(ir::Type::list(args[0].clone())),
        }),
        ("FileStream", "read") => Some(ir::Type::Function {
            params: vec![ir::Type::Int],
            ret: Box::new(ir::Type::Named {
                name: "Result".to_string(),
                args: vec![ir::Type::list(ir::Type::Int), ir::Type::named("FileError")],
            }),
        }),
        ("FileStream", "readToEnd") => Some(ir::Type::Function {
            params: Vec::new(),
            ret: Box::new(ir::Type::Named {
                name: "Result".to_string(),
                args: vec![ir::Type::list(ir::Type::Int), ir::Type::named("FileError")],
            }),
        }),
        ("FileStream", "seek") => Some(ir::Type::Function {
            params: vec![ir::Type::Int],
            ret: Box::new(ir::Type::Named {
                name: "Result".to_string(),
                args: vec![ir::Type::Int, ir::Type::named("FileError")],
            }),
        }),
        ("FileStream" | "TextFileReader", "close") => Some(ir::Type::Function {
            params: Vec::new(),
            ret: Box::new(ir::Type::Named {
                name: "Result".to_string(),
                args: vec![ir::Type::Unit, ir::Type::named("FileError")],
            }),
        }),
        ("TextFileReader", "readLine") => Some(ir::Type::Function {
            params: Vec::new(),
            ret: Box::new(ir::Type::Named {
                name: "Result".to_string(),
                args: vec![
                    ir::Type::option(ir::Type::Str),
                    ir::Type::named("FileError"),
                ],
            }),
        }),
        ("TextFileReader", "readToEnd") => Some(ir::Type::Function {
            params: Vec::new(),
            ret: Box::new(ir::Type::Named {
                name: "Result".to_string(),
                args: vec![ir::Type::Str, ir::Type::named("FileError")],
            }),
        }),
        _ => None,
    }
}

fn builtin_getter_type(receiver: &ir::Type, name: &str) -> Option<ir::Type> {
    if matches!(receiver, ir::Type::Str) {
        return match name {
            "size" => Some(ir::Type::Int),
            "isEmpty" | "nonEmpty" => Some(ir::Type::Bool),
            _ => None,
        };
    }

    let ir::Type::Named {
        name: type_name,
        args,
    } = receiver
    else {
        return None;
    };
    let item = args.first().cloned().unwrap_or(ir::Type::Unknown);

    match (type_name.as_str(), name) {
        ("Str", "size") => Some(ir::Type::Int),
        ("Str", "isEmpty" | "nonEmpty") => Some(ir::Type::Bool),
        ("Option", "isSet" | "isDefined" | "isSuccess" | "isEmpty")
        | ("Result", "isOk" | "isSuccess" | "isErr")
        | ("Either", "isLeft" | "isRight" | "isSuccess") => Some(ir::Type::Bool),
        ("Vector" | "LinkedList" | "Array", "head" | "first" | "last") => {
            Some(ir::Type::option(item))
        }
        ("Vector" | "LinkedList" | "Array" | "Set" | "Map", "size") => Some(ir::Type::Int),
        ("Vector" | "LinkedList" | "Set", "isEmpty" | "nonEmpty") => Some(ir::Type::Bool),
        ("Type", "name" | "qualifiedName")
        | (
            "ClassType" | "ShapeType" | "EnumType" | "InterfaceType" | "ObjectType"
            | "AnnotationType",
            "name" | "qualifiedName",
        ) => Some(ir::Type::option(ir::Type::Str)),
        ("Type", "kind")
        | (
            "ClassType" | "ShapeType" | "EnumType" | "InterfaceType" | "ObjectType"
            | "AnnotationType",
            "kind",
        ) => Some(ir::Type::named("TypeKind")),
        ("AnnotationValue", "name") | ("Field" | "Method" | "Param" | "EnumCase", "name") => {
            Some(ir::Type::Str)
        }
        ("ClassType" | "ShapeType" | "ObjectType" | "AnnotationType", "fields")
        | ("EnumCase", "fields") => Some(ir::Type::list(ir::Type::named("Field"))),
        ("ClassType" | "ShapeType" | "EnumType" | "InterfaceType" | "ObjectType", "methods") => {
            Some(ir::Type::list(ir::Type::named("Method")))
        }
        ("Method", "params") => Some(ir::Type::list(ir::Type::named("Param"))),
        ("EnumType", "cases") => Some(ir::Type::list(ir::Type::named("EnumCase"))),
        ("Field", "fieldType") | ("Method", "returnType") | ("Param", "paramType") => {
            Some(ir_exact_runtime_type(ir::Type::Unknown))
        }
        ("Field", "isPrivate") => Some(ir::Type::Bool),
        ("FileStream" | "TextFileReader", "path") => Some(ir::Type::Str),
        ("FileStream", "position") => Some(ir::Type::Int),
        ("FileStream" | "TextFileReader", "closed") => Some(ir::Type::Bool),
        _ => None,
    }
}

fn is_known_getter_name(name: &str) -> bool {
    matches!(
        name,
        "hasNext"
            | "isSet"
            | "isDefined"
            | "isSuccess"
            | "isEmpty"
            | "nonEmpty"
            | "isOk"
            | "isErr"
            | "isLeft"
            | "isRight"
    )
}

fn is_annotated_metadata_type(ty: &ir::Type) -> bool {
    matches!(
        ty,
        ir::Type::Named { name, .. }
            if matches!(
                name.as_str(),
                "Type"
                    | "ClassType"
                    | "ShapeType"
                    | "EnumType"
                    | "InterfaceType"
                    | "ObjectType"
                    | "AnnotationType"
                    | "Field"
                    | "Method"
                    | "EnumCase"
            )
    )
}

fn inferred_storage_type(ty: ir::Type) -> ir::Type {
    if contains_type_param(&ty) {
        ir::Type::Unknown
    } else {
        ty
    }
}

fn contains_type_param(ty: &ir::Type) -> bool {
    match ty {
        ir::Type::TypeParam(_) => true,
        ir::Type::Named { args, .. } | ir::Type::Tuple(args) | ir::Type::Union(args) => {
            args.iter().any(contains_type_param)
        }
        ir::Type::Record(fields) => fields.iter().any(|field| contains_type_param(&field.ty)),
        ir::Type::Function { params, ret } => {
            params.iter().any(contains_type_param) || contains_type_param(ret)
        }
        ir::Type::Unknown
        | ir::Type::Never
        | ir::Type::Unit
        | ir::Type::Bool
        | ir::Type::Int
        | ir::Type::Float
        | ir::Type::Str => false,
    }
}

fn universal_member_type(name: &str) -> Option<ir::Type> {
    match name {
        "toStr" => Some(ir::Type::Function {
            params: Vec::new(),
            ret: Box::new(ir::Type::Str),
        }),
        "equals" => Some(ir::Type::Function {
            params: vec![ir::Type::named("Any")],
            ret: Box::new(ir::Type::Bool),
        }),
        "hash" => Some(ir::Type::Function {
            params: Vec::new(),
            ret: Box::new(ir::Type::Int),
        }),
        _ => None,
    }
}

fn function_type_returns_unknown(ty: &ir::Type) -> bool {
    matches!(
        ty,
        ir::Type::Function { ret, .. } if matches!(ret.as_ref(), ir::Type::Unknown)
    )
}

fn method_call_arity_score(function: &ir::Function, args: &[core::CallArg]) -> Option<usize> {
    let param_indices = source_param_indices(function);
    let param_count = param_indices.len();
    let mut slots = vec![0usize; param_count];
    let mut positional_index = 0usize;
    let mut used_variadic_positionally = false;

    for arg in args {
        if let Some(name) = &arg.name {
            let index = param_indices.iter().position(|param_index| {
                function.params.get(*param_index).is_some_and(|param| {
                    function
                        .locals
                        .get(param.0)
                        .is_some_and(|local| local.name == *name)
                })
            })?;
            if slots[index] > 0 {
                return None;
            }
            slots[index] = 1;
            continue;
        }

        while positional_index < param_count
            && !param_indices
                .get(positional_index)
                .and_then(|index| function.param_variadic.get(*index))
                .copied()
                .unwrap_or(false)
            && slots[positional_index] > 0
        {
            positional_index += 1;
        }

        let last_is_variadic = param_indices
            .last()
            .and_then(|index| function.param_variadic.get(*index))
            .copied()
            .unwrap_or(false);
        if last_is_variadic && positional_index >= param_count.saturating_sub(1) {
            let slot = slots.last_mut()?;
            *slot += 1;
            used_variadic_positionally = true;
        } else if positional_index < param_count {
            if slots[positional_index] > 0 {
                return None;
            }
            slots[positional_index] = 1;
            if !param_indices
                .get(positional_index)
                .and_then(|index| function.param_variadic.get(*index))
                .copied()
                .unwrap_or(false)
            {
                positional_index += 1;
            }
        } else {
            return None;
        }
    }

    let mut omitted_defaults = 0usize;
    for (source_index, slot_count) in slots.iter().enumerate() {
        let index = param_indices[source_index];
        let variadic = function.param_variadic.get(index).copied().unwrap_or(false);
        let has_default = function
            .param_defaults
            .get(index)
            .is_some_and(|default| default.is_some());
        if !variadic && !has_default && *slot_count == 0 {
            return None;
        }
        if !variadic && *slot_count > 1 {
            return None;
        }
        if variadic && *slot_count == 0 {
            omitted_defaults += 1;
        } else if has_default && *slot_count == 0 {
            omitted_defaults += 1;
        }
    }

    let has_variadic = param_indices.iter().any(|index| {
        function
            .param_variadic
            .get(*index)
            .copied()
            .unwrap_or(false)
    });
    if !has_variadic && args.len() == param_count {
        return Some(400 + args.len());
    }
    if !has_variadic {
        return Some(300usize.saturating_sub(omitted_defaults));
    }
    if used_variadic_positionally {
        return Some(200 + args.len());
    }
    Some(100usize.saturating_sub(omitted_defaults))
}

fn ir_type_subst(ty: &ir::TypeDef, args: &[ir::Type]) -> HashMap<String, ir::Type> {
    ty.type_params
        .iter()
        .cloned()
        .zip(args.iter().cloned())
        .collect()
}

fn substitute_ir_type(ty: &ir::Type, subst: &HashMap<String, ir::Type>) -> ir::Type {
    match ty {
        ir::Type::TypeParam(name) => subst.get(name).cloned().unwrap_or_else(|| ty.clone()),
        ir::Type::Named { name, args } => ir::Type::Named {
            name: name.clone(),
            args: args
                .iter()
                .map(|arg| substitute_ir_type(arg, subst))
                .collect(),
        },
        ir::Type::Union(members) => ir::Type::Union(
            members
                .iter()
                .map(|member| substitute_ir_type(member, subst))
                .collect(),
        ),
        ir::Type::Tuple(items) => ir::Type::Tuple(
            items
                .iter()
                .map(|item| substitute_ir_type(item, subst))
                .collect(),
        ),
        ir::Type::Record(fields) => ir::Type::Record(
            fields
                .iter()
                .map(|field| ir::NamedType {
                    name: field.name.clone(),
                    ty: substitute_ir_type(&field.ty, subst),
                })
                .collect(),
        ),
        ir::Type::Function { params, ret } => ir::Type::Function {
            params: params
                .iter()
                .map(|param| substitute_ir_type(param, subst))
                .collect(),
            ret: Box::new(substitute_ir_type(ret, subst)),
        },
        ir::Type::Unknown
        | ir::Type::Never
        | ir::Type::Unit
        | ir::Type::Bool
        | ir::Type::Int
        | ir::Type::Float
        | ir::Type::Str => ty.clone(),
    }
}

fn ir_type_is_hashable(
    ty: &ir::Type,
    owner: &ir::TypeDef,
    types: &[ir::TypeDef],
    seen: &mut HashSet<String>,
) -> bool {
    match ty {
        ir::Type::Unit | ir::Type::Bool | ir::Type::Int | ir::Type::Float | ir::Type::Str => true,
        ir::Type::Named { name, args }
            if args.is_empty()
                && matches!(
                    name.as_str(),
                    "Bool" | "Float" | "Int" | "Rune" | "Str" | "Unit"
                ) =>
        {
            true
        }
        ir::Type::TypeParam(name) => owner.generic_conditions.iter().any(|condition| {
            matches!(
                condition,
                ir::GenericCondition::Bound {
                    subject: ir::Type::TypeParam(subject),
                    bound: ir::Type::Named { name: bound, args }
                } if subject == name
                    && bound == "Hashed"
                    && args.len() == 1
                    && matches!(&args[0], ir::Type::TypeParam(bound_param) if bound_param == name)
            )
        }),
        ir::Type::Record(fields) => fields
            .iter()
            .all(|field| ir_type_is_hashable(&field.ty, owner, types, seen)),
        ir::Type::Tuple(items) | ir::Type::Union(items) => items
            .iter()
            .all(|item| ir_type_is_hashable(item, owner, types, seen)),
        ir::Type::Named { name, args } => {
            if let Some((owner_name, case_name)) = enum_case_view_parts(name) {
                let Some(definition) = types.iter().find(|candidate| candidate.name == owner_name)
                else {
                    return false;
                };
                return ir_enum_type_is_hashable(definition, args, Some(case_name), types, seen);
            }
            let Some(definition) = types.iter().find(|candidate| candidate.name == *name) else {
                return false;
            };
            match definition.kind {
                ast::TypeKind::Enum => {
                    ir_enum_type_is_hashable(definition, args, None, types, seen)
                }
                ast::TypeKind::Object => true,
                ast::TypeKind::Record => {
                    let key = format!("{}<{args:?}>", definition.name);
                    if !seen.insert(key.clone()) {
                        return true;
                    }
                    let subst = definition
                        .type_params
                        .iter()
                        .cloned()
                        .zip(args.iter().cloned())
                        .collect::<HashMap<_, _>>();
                    let hashable = definition.fields.iter().all(|field| {
                        let field_ty = substitute_ir_type(&field.ty, &subst);
                        ir_type_is_hashable(&field_ty, definition, types, seen)
                    });
                    seen.remove(&key);
                    hashable
                }
                ast::TypeKind::Class => ir_type_has_hashed_bound(definition, ty, types, seen),
                ast::TypeKind::Annotation | ast::TypeKind::Interface => false,
            }
        }
        ir::Type::Unknown | ir::Type::Never | ir::Type::Function { .. } => false,
    }
}

fn ir_enum_type_is_hashable(
    definition: &ir::TypeDef,
    args: &[ir::Type],
    case_name: Option<&str>,
    types: &[ir::TypeDef],
    seen: &mut HashSet<String>,
) -> bool {
    let key = format!(
        "union:{}<{args:?}>:{}",
        definition.name,
        case_name.unwrap_or("*")
    );
    if !seen.insert(key.clone()) {
        return true;
    }
    let subst = definition
        .type_params
        .iter()
        .cloned()
        .zip(args.iter().cloned())
        .collect::<HashMap<_, _>>();
    let shared_fields_hashable = definition.fields.iter().all(|field| {
        let field_ty = substitute_ir_type(&field.ty, &subst);
        ir_type_is_hashable(&field_ty, definition, types, seen)
    });
    let cases_hashable = match case_name {
        Some(case_name) => definition
            .enum_cases
            .iter()
            .find(|case| case.name == case_name)
            .is_some_and(|case| {
                case.fields.iter().all(|field| {
                    let field_ty = substitute_ir_type(&field.ty, &subst);
                    ir_type_is_hashable(&field_ty, definition, types, seen)
                })
            }),
        None => definition.enum_cases.iter().all(|case| {
            case.fields.iter().all(|field| {
                let field_ty = substitute_ir_type(&field.ty, &subst);
                ir_type_is_hashable(&field_ty, definition, types, seen)
            })
        }),
    };
    seen.remove(&key);
    shared_fields_hashable && cases_hashable
}

fn ir_type_has_hashed_bound(
    ty: &ir::TypeDef,
    actual: &ir::Type,
    types: &[ir::TypeDef],
    seen: &mut HashSet<String>,
) -> bool {
    let key = format!("hashed:{}:{actual:?}", ty.name);
    if !seen.insert(key.clone()) {
        return false;
    }
    let subst = match actual {
        ir::Type::Named { args, .. } => ty
            .type_params
            .iter()
            .cloned()
            .zip(args.iter().cloned())
            .collect::<HashMap<_, _>>(),
        _ => HashMap::new(),
    };
    let found = ty.with_bounds.iter().any(|bound| {
        let bound = substitute_ir_type(bound, &subst);
        let ir::Type::Named { name, args } = &bound else {
            return false;
        };
        if name == "Hashed" {
            return args.len() == 1 && args.first() == Some(actual);
        }
        types
            .iter()
            .find(|candidate| candidate.name == *name)
            .is_some_and(|parent| ir_type_has_hashed_bound(parent, &bound, types, seen))
    });
    seen.remove(&key);
    found
}

fn infer_ir_type_subst(
    expected: &ir::Type,
    actual: &ir::Type,
    subst: &mut HashMap<String, ir::Type>,
) {
    match expected {
        ir::Type::TypeParam(name) => {
            if matches!(actual, ir::Type::Unknown | ir::Type::TypeParam(_)) {
                return;
            }
            subst
                .entry(name.clone())
                .and_modify(|existing| *existing = join_ir_types(existing.clone(), actual.clone()))
                .or_insert_with(|| actual.clone());
        }
        ir::Type::Named {
            name: expected_name,
            args: expected_args,
        } => {
            if let ir::Type::Named {
                name: actual_name,
                args: actual_args,
            } = actual
            {
                if expected_name == actual_name && expected_args.len() == actual_args.len() {
                    for (expected_arg, actual_arg) in expected_args.iter().zip(actual_args.iter()) {
                        infer_ir_type_subst(expected_arg, actual_arg, subst);
                    }
                }
            }
        }
        ir::Type::Union(expected_members) => {
            if let ir::Type::Union(actual_members) = actual {
                for (expected_member, actual_member) in
                    expected_members.iter().zip(actual_members.iter())
                {
                    infer_ir_type_subst(expected_member, actual_member, subst);
                }
            }
        }
        ir::Type::Tuple(expected_items) => {
            if let ir::Type::Tuple(actual_items) = actual {
                for (expected_item, actual_item) in expected_items.iter().zip(actual_items.iter()) {
                    infer_ir_type_subst(expected_item, actual_item, subst);
                }
            }
        }
        ir::Type::Record(expected_fields) => {
            if let ir::Type::Record(actual_fields) = actual {
                for expected_field in expected_fields {
                    if let Some(actual_field) = actual_fields
                        .iter()
                        .find(|actual_field| actual_field.name == expected_field.name)
                    {
                        infer_ir_type_subst(&expected_field.ty, &actual_field.ty, subst);
                    }
                }
            }
        }
        ir::Type::Function {
            params: expected_params,
            ret: expected_ret,
        } => {
            if let ir::Type::Function {
                params: actual_params,
                ret: actual_ret,
            } = actual
            {
                if expected_params.len() == actual_params.len() {
                    for (expected_param, actual_param) in
                        expected_params.iter().zip(actual_params.iter())
                    {
                        infer_ir_type_subst(expected_param, actual_param, subst);
                    }
                }
                infer_ir_type_subst(expected_ret, actual_ret, subst);
            }
        }
        ir::Type::Unknown
        | ir::Type::Never
        | ir::Type::Unit
        | ir::Type::Bool
        | ir::Type::Int
        | ir::Type::Float
        | ir::Type::Str => {}
    }
}

fn contextualize_inferred_ir_type(actual: ir::Type, expected: &ir::Type) -> ir::Type {
    match (actual, expected) {
        (ir::Type::Unknown, ir::Type::TypeParam(_)) => ir::Type::Unknown,
        (ir::Type::Unknown, expected) => erase_ir_type_params(expected),
        (
            ir::Type::Named {
                name: actual_name,
                args: actual_args,
            },
            ir::Type::Named {
                name: expected_name,
                args: expected_args,
            },
        ) if actual_name == *expected_name && actual_args.len() == expected_args.len() => {
            ir::Type::Named {
                name: actual_name,
                args: actual_args
                    .into_iter()
                    .zip(expected_args.iter())
                    .map(|(actual, expected)| contextualize_inferred_ir_type(actual, expected))
                    .collect(),
            }
        }
        (
            ir::Type::Function {
                params: actual_params,
                ret: actual_ret,
            },
            ir::Type::Function {
                params: expected_params,
                ret: expected_ret,
            },
        ) if actual_params.len() == expected_params.len() => ir::Type::Function {
            params: actual_params
                .into_iter()
                .zip(expected_params.iter())
                .map(|(actual, expected)| contextualize_inferred_ir_type(actual, expected))
                .collect(),
            ret: Box::new(contextualize_inferred_ir_type(*actual_ret, expected_ret)),
        },
        (actual, _) => actual,
    }
}

fn erase_ir_type_params(ty: &ir::Type) -> ir::Type {
    match ty {
        ir::Type::TypeParam(_) => ir::Type::Unknown,
        ir::Type::Named { name, args } => ir::Type::Named {
            name: name.clone(),
            args: args.iter().map(erase_ir_type_params).collect(),
        },
        ir::Type::Union(members) => {
            ir::Type::Union(members.iter().map(erase_ir_type_params).collect())
        }
        ir::Type::Tuple(items) => ir::Type::Tuple(items.iter().map(erase_ir_type_params).collect()),
        ir::Type::Record(fields) => ir::Type::Record(
            fields
                .iter()
                .map(|field| ir::NamedType {
                    name: field.name.clone(),
                    ty: erase_ir_type_params(&field.ty),
                })
                .collect(),
        ),
        ir::Type::Function { params, ret } => ir::Type::Function {
            params: params.iter().map(erase_ir_type_params).collect(),
            ret: Box::new(erase_ir_type_params(ret)),
        },
        _ => ty.clone(),
    }
}

fn generic_call_type_arg_refs_from_expr(expr: &Expr) -> Option<Vec<TypeRef>> {
    match expr {
        Expr::TupleLiteral { items, .. } => items
            .iter()
            .map(generic_call_type_ref_from_expr)
            .collect::<Option<Vec<_>>>(),
        _ => Some(vec![generic_call_type_ref_from_expr(expr)?]),
    }
}

fn generic_call_type_ref_from_expr(expr: &Expr) -> Option<TypeRef> {
    match expr {
        Expr::Identifier { name, span } => Some(TypeRef::Named {
            name: name.clone(),
            args: Vec::new(),
            span: *span,
        }),
        Expr::Member { span, .. } => Some(TypeRef::Named {
            name: expr_path(expr)?.join("."),
            args: Vec::new(),
            span: *span,
        }),
        Expr::Index {
            receiver,
            index,
            span,
        } => {
            let TypeRef::Named { name, .. } = generic_call_type_ref_from_expr(receiver)? else {
                return None;
            };
            Some(TypeRef::Named {
                name,
                args: generic_call_type_arg_refs_from_expr(index)?,
                span: *span,
            })
        }
        _ => None,
    }
}

fn lower_field_initializer_constant(initializer: Option<&ast::Expr>) -> Option<ir::Constant> {
    match initializer? {
        ast::Expr::Group { inner, .. } => lower_field_initializer_constant(Some(inner)),
        ast::Expr::Integer { raw, .. } => Some(ir::Constant::Int(raw.parse::<i64>().unwrap_or(0))),
        ast::Expr::Float { raw, .. } => {
            Some(ir::Constant::Float(raw.parse::<f64>().unwrap_or(0.0)))
        }
        ast::Expr::String { raw, .. } => Some(ir::Constant::String(raw.clone())),
        ast::Expr::Bool { value, .. } => Some(ir::Constant::Bool(*value)),
        ast::Expr::Unit { .. } => Some(ir::Constant::Unit),
        ast::Expr::ListLiteral { items, .. } => items
            .iter()
            .map(|item| lower_field_initializer_constant(Some(item)))
            .collect::<Option<Vec<_>>>()
            .map(ir::Constant::List),
        _ => None,
    }
}

fn lower_param_default_constant(initializer: &ast::Expr) -> Option<ir::Constant> {
    match initializer {
        ast::Expr::Group { inner, .. } => lower_param_default_constant(inner),
        ast::Expr::Identifier { name, .. } if name == "None" => Some(ir::Constant::OptionNone),
        other => lower_field_initializer_constant(Some(other)),
    }
}

fn default_constant_for_type(ty: &ir::Type) -> ir::Constant {
    match ty {
        ir::Type::Unit => ir::Constant::Unit,
        ir::Type::Bool => ir::Constant::Bool(false),
        ir::Type::Int => ir::Constant::Int(0),
        ir::Type::Float => ir::Constant::Float(0.0),
        ir::Type::Str => ir::Constant::String(String::new()),
        ir::Type::Named { name, .. } if name == "Bool" => ir::Constant::Bool(false),
        ir::Type::Named { name, .. } if matches!(name.as_str(), "Int" | "Rune") => {
            ir::Constant::Int(0)
        }
        ir::Type::Named { name, .. } if name == "Float" => ir::Constant::Float(0.0),
        ir::Type::Named { name, .. } if name == "Str" => ir::Constant::String(String::new()),
        _ => ir::Constant::Unit,
    }
}

fn named_type_name(reference: &TypeRef) -> Option<&str> {
    match reference {
        TypeRef::Named { name, .. } => Some(name.as_str()),
        _ => None,
    }
}

fn expr_path(expr: &Expr) -> Option<Vec<String>> {
    match expr {
        Expr::Identifier { name, .. } => Some(vec![name.clone()]),
        Expr::Member { receiver, name, .. } => {
            let mut path = expr_path(receiver)?;
            path.push(name.clone());
            Some(path)
        }
        _ => None,
    }
}

fn callable_reference_call_expr(reference: &Expr, param_names: &[String], span: Span) -> Expr {
    Expr::Call {
        callee: Box::new(reference.clone()),
        args: param_names
            .iter()
            .map(|name| core::CallArg {
                name: None,
                ty: None,
                value: Expr::Identifier {
                    name: name.clone(),
                    span,
                },
                span,
            })
            .collect(),
        style: core::CallStyle::Paren,
        span,
    }
}

fn list_literal_has_spread(items: &[Expr]) -> bool {
    items.iter().any(|item| matches!(item, Expr::Spread { .. }))
}

fn explicit_any_widening_value(expr: &Expr) -> Option<&Expr> {
    let Expr::Call {
        callee,
        args,
        style: core::CallStyle::Paren,
        ..
    } = expr
    else {
        return None;
    };
    if !matches!(callee.as_ref(), Expr::Identifier { name, .. } if name == "Any")
        || args.len() != 1
        || args[0].name.is_some()
        || args[0].ty.is_some()
    {
        return None;
    }
    Some(&args[0].value)
}

fn is_named_runtime_value_path(program: &ir::Program, path: &[String]) -> bool {
    if path.is_empty() {
        return false;
    }

    let qualified = path.join(".");
    if program
        .types
        .iter()
        .any(|ty| ty.name == qualified && ty.kind == ast::TypeKind::Object)
    {
        return true;
    }

    if path.len() >= 2 && explicit_enum_case_value_exists(program, &path[0], &path[1]) {
        return true;
    }

    if matches!(path, [owner, case] if owner == "SeekFrom" && matches!(case.as_str(), "Start" | "Current" | "End"))
    {
        return true;
    }

    if single_type_exists(program, &path[0]) {
        return true;
    }

    path.len() == 1
        && (builtin_zero_arg_value_name(&path[0])
            || unique_bare_enum_case_value_exists(program, &path[0]))
}

fn is_named_runtime_callee_path(program: &ir::Program, path: &[String]) -> bool {
    if path.is_empty() {
        return false;
    }
    if declared_type_exists(program, &path.join(".")) {
        return true;
    }
    if matches!(path, [owner, method] if ((owner == "Int" || owner == "Float") && method == "parse")
        || (owner == "Option" && method == "when"))
    {
        return true;
    }
    if path.len() == 1 {
        return runtime_callable_root_name(&path[0])
            || declared_type_exists(program, &path[0])
            || unique_bare_enum_case_exists(program, &path[0]);
    }
    explicit_enum_case_exists(program, &path[0], &path[1])
        || single_type_exists(program, &path[0])
        || runtime_callable_root_name(&path[0])
        || declared_type_exists(program, &path[0])
}

fn declared_type_exists(program: &ir::Program, name: &str) -> bool {
    program.types.iter().any(|ty| ty.name == name)
}

// Standard-library factory signatures are checked from source declarations.
// This list only routes the checked calls to their runtime ABI entry points.
fn runtime_callable_root_name(name: &str) -> bool {
    matches!(
        name,
        "OS" | "Math"
            | "File"
            | "SeekFrom"
            | "Range"
            | "IntRange"
            | "Vector"
            | "LinkedList"
            | "Array"
            | "Set"
            | "Map"
            | "Some"
            | "None"
            | "Ok"
            | "Err"
            | "Left"
            | "Right"
    )
}

fn runtime_collection_constructor_name(name: &str) -> bool {
    matches!(name, "Array" | "LinkedList" | "Map" | "Set" | "Vector")
}

fn single_type_exists(program: &ir::Program, name: &str) -> bool {
    program
        .types
        .iter()
        .any(|ty| ty.kind == ast::TypeKind::Object && ty.name == name)
}

fn builtin_zero_arg_value_name(name: &str) -> bool {
    matches!(name, "None")
}

fn core_pattern_case_fields(
    path: &[String],
    scrutinee_ty: Option<&ir::Type>,
) -> Option<Vec<String>> {
    let case_name = path.last()?.as_str();
    let inferred_owner = if path.len() >= 2 {
        Some(path.get(path.len() - 2)?.as_str())
    } else if let Some(ir::Type::Named { name, .. }) = scrutinee_ty {
        Some(name.split("::").next().unwrap_or(name))
    } else {
        None
    };
    // The lifted wrapper cases are language-level bare patterns. Their scrutinee can
    // still carry an unresolved generic owner while nested patterns are lowered.
    let owner = match case_name {
        "Some" | "None" if path.len() == 1 => "Option",
        "Ok" | "Err" if path.len() == 1 => "Result",
        "Left" | "Right" if path.len() == 1 => "Either",
        _ => inferred_owner?,
    };

    let fields = match (owner, case_name) {
        ("Option", "Some") | ("Result", "Ok") | ("Either", "Left" | "Right") => &["value"][..],
        ("Option", "None") | ("SeekFrom", "Start" | "Current" | "End") => &[],
        ("Result", "Err") => &["error"],
        ("FileError", "NotFound" | "AccessDenied" | "Closed") => &["path"],
        ("FileError", "InvalidEncoding") => &["path", "offset"],
        ("FileError", "IoFailure") => &["operation", "path", "message"],
        _ => return None,
    };
    Some(fields.iter().map(|field| (*field).to_string()).collect())
}

fn explicit_enum_case_exists(program: &ir::Program, type_name: &str, case_name: &str) -> bool {
    program
        .types
        .iter()
        .filter(|ty| ty.kind == ast::TypeKind::Enum && ty.name == type_name)
        .flat_map(|ty| ty.enum_cases.iter())
        .any(|case| case.name == case_name)
}

fn explicit_enum_case_value_exists(
    program: &ir::Program,
    type_name: &str,
    case_name: &str,
) -> bool {
    program
        .types
        .iter()
        .filter(|ty| ty.kind == ast::TypeKind::Enum && ty.name == type_name)
        .flat_map(|ty| ty.enum_cases.iter())
        .any(|case| case.name == case_name && enum_case_is_value(case))
}

fn unique_bare_enum_case_value_exists(program: &ir::Program, case_name: &str) -> bool {
    program
        .types
        .iter()
        .filter(|ty| ty.kind == ast::TypeKind::Enum)
        .flat_map(|ty| ty.enum_cases.iter())
        .filter(|case| case.name == case_name && enum_case_is_value(case))
        .count()
        == 1
}

fn unique_bare_enum_case_owner<'a>(program: &'a ir::Program, case_name: &str) -> Option<&'a str> {
    let mut owners = program.types.iter().filter_map(|ty| {
        (ty.kind == ast::TypeKind::Enum && ty.enum_cases.iter().any(|case| case.name == case_name))
            .then_some(ty.name.as_str())
    });
    let owner = owners.next()?;
    owners.next().is_none().then_some(owner)
}

fn unique_bare_enum_case_exists(program: &ir::Program, case_name: &str) -> bool {
    program
        .types
        .iter()
        .filter(|ty| ty.kind == ast::TypeKind::Enum)
        .flat_map(|ty| ty.enum_cases.iter())
        .filter(|case| case.name == case_name)
        .count()
        == 1
}

fn enum_case_is_value(case: &ir::EnumCase) -> bool {
    case.fields.is_empty() || case.fields.iter().all(|field| field.initializer.is_some())
}

fn enum_case_pattern_fields(
    owner: &ir::TypeDef,
    case: &ir::EnumCase,
    arity: usize,
) -> Option<Vec<String>> {
    let shared_fields = owner
        .fields
        .iter()
        .map(|field| field.name.as_str())
        .collect::<HashSet<_>>();
    let extractable = case
        .fields
        .iter()
        .filter(|field| !shared_fields.contains(field.name.as_str()))
        .collect::<Vec<_>>();
    match (arity, extractable.as_slice()) {
        (0, []) => Some(Vec::new()),
        (1, [field]) => Some(vec![field.name.clone()]),
        _ => None,
    }
}

fn enum_case_view_name(owner: &str, case_name: &str) -> String {
    format!("{owner}::{case_name}")
}

fn enum_case_view_parts(name: &str) -> Option<(&str, &str)> {
    let (owner, case_name) = name.split_once("::")?;
    (!owner.is_empty() && !case_name.is_empty()).then_some((owner, case_name))
}

fn builtin_enum_case_view_field_type(
    owner: &str,
    case_name: &str,
    field_name: &str,
    args: &[ir::Type],
) -> Option<ir::Type> {
    match (owner, case_name, field_name, args) {
        ("Option", "Some", "value", [value]) => Some(value.clone()),
        ("Result", "Ok", "value", [value, _]) => Some(value.clone()),
        ("Result", "Err", "error", [_, error]) => Some(error.clone()),
        ("Either", "Left", "value", [left, _]) => Some(left.clone()),
        ("Either", "Right", "value", [_, right]) => Some(right.clone()),
        _ => None,
    }
}

fn arrange_named_call_args<'a>(
    params: &[String],
    args: &'a [core::CallArg],
) -> Option<Vec<&'a core::CallArg>> {
    let mut slots = vec![None; params.len()];
    let mut positional_index = 0usize;
    for arg in args {
        if let Some(name) = &arg.name {
            let index = params.iter().position(|param| param == name)?;
            if slots[index].is_some() {
                return None;
            }
            slots[index] = Some(arg);
            continue;
        }

        while positional_index < params.len() && slots[positional_index].is_some() {
            positional_index += 1;
        }
        if positional_index >= params.len() {
            return None;
        }
        slots[positional_index] = Some(arg);
        positional_index += 1;
    }

    Some(slots.into_iter().flatten().collect())
}

fn call_uses_structural_record_arg(args: &[core::CallArg], style: core::CallStyle) -> bool {
    style == core::CallStyle::Brace
        && matches!(
            args,
            [core::CallArg {
                name: None,
                value: Expr::RecordLiteral { .. },
                ..
            }]
        )
}

fn brace_record_constructor_args(args: &[core::CallArg]) -> Option<Vec<core::CallArg>> {
    let [
        core::CallArg {
            name: None,
            value: Expr::RecordLiteral { fields, values, .. },
            ..
        },
    ] = args
    else {
        return None;
    };

    if values.is_empty() && fields.iter().all(|field| field.name.is_some()) {
        return Some(fields.clone());
    }

    None
}

fn param_names_from_function(function: &ir::Function) -> Vec<String> {
    function
        .params
        .iter()
        .filter_map(|param| function.locals.get(param.0))
        .filter(|local| !is_reified_type_param_local(&local.name))
        .map(|local| local.name.clone())
        .collect()
}

fn reified_type_param_local_name(name: &str) -> String {
    format!("__type_{name}")
}

fn is_reified_type_param_local(name: &str) -> bool {
    name.starts_with("__type_")
}

fn source_param_indices(function: &ir::Function) -> Vec<usize> {
    function
        .params
        .iter()
        .enumerate()
        .filter_map(|(index, param)| {
            function
                .locals
                .get(param.0)
                .is_some_and(|local| !is_reified_type_param_local(&local.name))
                .then_some(index)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{SourceFile, lex, parse_program};

    fn parse_inline(src: &str) -> ast::Program {
        let file = SourceFile::new("test.lum", src);
        let lexed = lex(&file);
        assert!(lexed.diagnostics.is_empty(), "{:#?}", lexed.diagnostics);
        let parsed = parse_program(&lexed.tokens);
        assert!(parsed.diagnostics.is_empty(), "{:#?}", parsed.diagnostics);
        parsed.program.expect("program")
    }

    #[test]
    fn lowers_top_level_functions_and_entry() {
        let program = parse_inline(
            r#"
            def add(a Int, b Int) Int = a + b

            def main() Int {
                value Int = add(1, 2)
                return value
            }
            "#,
        );

        let lowered = lower_program(&program);
        assert!(lowered.diagnostics.is_empty(), "{:#?}", lowered.diagnostics);
        let ir = lowered.program.expect("ir program");
        assert_eq!(ir.functions.len(), 2);
        assert_eq!(ir.entry, Some(ir::FunctionId(1)));
        let main = ir.function(ir::FunctionId(1)).expect("main function");
        assert_eq!(main.params.len(), 0);
        assert!(!main.blocks.is_empty());
    }

    #[test]
    fn preserves_declared_union_and_alternative_annotations() {
        let program = parse_inline(
            r#"
            annotation Serializable {}
            annotation Payload {}

            @Serializable
            type Outcome =
                @Payload class Success { value Str }
                | @Payload object Cancelled {}
            "#,
        );

        let lowered = lower_program(&program);
        assert!(lowered.diagnostics.is_empty(), "{:#?}", lowered.diagnostics);
        let ir = lowered.program.expect("ir program");
        let outcome = ir.types.iter().find(|ty| ty.name == "Outcome").unwrap();
        assert_eq!(outcome.annotations[0].name, "Serializable");
        assert_eq!(outcome.enum_cases.len(), 2);
        assert_eq!(outcome.enum_cases[0].annotations[0].name, "Payload");
        assert_eq!(outcome.enum_cases[1].annotations[0].name, "Payload");
    }

    #[test]
    fn lowers_anonymous_shape_literal_as_record() {
        let program = parse_inline(
            r#"
            def main() Int {
                session { start Int, end Int } = { start: 10, end: 30 }
                return session.end - session.start
            }
            "#,
        );

        let lowered = lower_program(&program);
        assert!(lowered.diagnostics.is_empty(), "{:#?}", lowered.diagnostics);
        let ir = lowered.program.expect("ir program");
        let main = ir
            .function(ir.entry.expect("entry function"))
            .expect("main");
        assert!(
            main.blocks
                .iter()
                .flat_map(|block| &block.statements)
                .any(|statement| matches!(
                    &statement.kind,
                    ir::StatementKind::Assign {
                        value: ir::RValue::Record(fields),
                        ..
                    } if fields.iter().map(|field| field.name.as_str()).eq(["start", "end"])
                ))
        );
    }

    #[test]
    fn expands_transparent_aliases_throughout_lowered_ir() {
        let program = parse_inline(
            r#"
            type UserId = Int
            type Names = [Str]
            type Labeler = fn(UserId) Str

            class Holder {
                id UserId
                names Names
            }

            current UserId = 1

            def main() Unit {
                id UserId = current
                names Names = ["Ada"]
                label Labeler = value => value.toStr()
                println(label(id))
                println(names[0])
            }
            "#,
        );

        let lowered = lower_program(&program);
        assert!(lowered.diagnostics.is_empty(), "{:#?}", lowered.diagnostics);
        let ir = lowered.program.expect("ir program");

        assert_eq!(ir.globals[0].ty, ir::Type::named("Int"));
        let holder = ir.types.iter().find(|ty| ty.name == "Holder").unwrap();
        assert_eq!(holder.fields[0].ty, ir::Type::named("Int"));
        assert_eq!(
            holder.fields[1].ty,
            ir::Type::Named {
                name: "Vector".to_string(),
                args: vec![ir::Type::named("Str")],
            }
        );

        let main = ir
            .functions
            .iter()
            .find(|function| function.name == "main")
            .unwrap();
        let local = |name: &str| main.locals.iter().find(|local| local.name == name).unwrap();
        assert_eq!(local("id").ty, ir::Type::named("Int"));
        assert_eq!(
            local("names").ty,
            ir::Type::Named {
                name: "Vector".to_string(),
                args: vec![ir::Type::named("Str")],
            }
        );
        assert_eq!(
            local("label").ty,
            ir::Type::Function {
                params: vec![ir::Type::named("Int")],
                ret: Box::new(ir::Type::named("Str")),
            }
        );
    }

    #[test]
    fn lowers_union_variant_alias_as_variant_view() {
        let program = parse_inline(
            r#"
            type Payload =
                class Item { value Int }
                | object Empty {}

            def read(payload Payload) Int = match payload {
                case Payload.Item(item) as whole => whole.value + item
            }
            "#,
        );

        let lowered = lower_program(&program);
        assert!(lowered.diagnostics.is_empty(), "{:#?}", lowered.diagnostics);
        let ir = lowered.program.expect("ir program");
        let read = ir
            .functions
            .iter()
            .find(|function| function.name == "read")
            .unwrap();
        let whole = read
            .locals
            .iter()
            .find(|local| local.name == "whole")
            .unwrap();
        assert_eq!(
            whole.ty,
            ir::Type::Named {
                name: "Payload::Item".to_string(),
                args: Vec::new(),
            }
        );
    }

    #[test]
    fn lowers_if_and_while_into_cfg_blocks() {
        let program = parse_inline(
            r#"
            def main() Int {
                var total Int = 0
                if total < 1 {
                    total += 2
                } else {
                    total = 5
                }
                while total < 10 {
                    total += 1
                    break
                }
                return total
            }
            "#,
        );

        let lowered = lower_program(&program);
        assert!(lowered.diagnostics.is_empty(), "{:#?}", lowered.diagnostics);
        let ir = lowered.program.expect("ir program");
        let main = ir.entry.and_then(|id| ir.function(id)).expect("main");
        assert!(main.blocks.len() >= 6, "{:#?}", main.blocks);
        assert!(
            main.blocks
                .iter()
                .any(|block| matches!(block.terminator.kind, ir::TerminatorKind::Branch { .. }))
        );
    }

    #[test]
    fn lowers_extract_pattern_bindings_with_inner_type() {
        let program = parse_inline(
            r#"
            class Row {
            }

            def unwrap(maybe Option[Row]) Int {
                let row <- maybe else return 0
                1
            }
            "#,
        );

        let lowered = lower_program(&program);
        assert!(lowered.diagnostics.is_empty(), "{:#?}", lowered.diagnostics);
        let ir = lowered.program.expect("ir program");
        let unwrap = ir
            .functions
            .iter()
            .find(|function| function.name == "unwrap")
            .expect("unwrap function");
        let row = unwrap
            .locals
            .iter()
            .find(|local| local.name == "row")
            .expect("row binding");

        assert_eq!(row.ty, ir::Type::named("Row"));
    }

    #[test]
    fn lowers_types_and_methods() {
        let program = parse_inline(
            r#"
            class Counter {
                value Int


                def bump(delta Int) Int = this.value + delta
}

            "#,
        );

        let lowered = lower_program(&program);
        assert!(lowered.diagnostics.is_empty(), "{:#?}", lowered.diagnostics);
        let ir = lowered.program.expect("ir program");
        assert_eq!(ir.types.len(), 1);
        assert_eq!(ir.functions.len(), 1);
        let ty = &ir.types[0];
        assert_eq!(ty.fields.len(), 1);
        assert_eq!(ty.methods.len(), 1);
    }

    #[test]
    fn lowers_bare_method_calls_as_receiver_calls() {
        let program = parse_inline(
            r#"
            class Counter {
                value Int


                def add(delta Int) Int = this.value + delta
                def twice(delta Int) Int = add(delta) + add(delta)
}

            "#,
        );

        let lowered = lower_program(&program);
        assert!(lowered.diagnostics.is_empty(), "{:#?}", lowered.diagnostics);
        let ir = lowered.program.expect("ir program");
        let twice = ir
            .functions
            .iter()
            .find(|function| function.name == "twice")
            .expect("twice method");
        let add_calls = twice
            .blocks
            .iter()
            .flat_map(|block| block.statements.iter())
            .filter(|stmt| {
                matches!(
                    &stmt.kind,
                    ir::StatementKind::Assign {
                        value:
                            ir::RValue::Call {
                                callee: ir::Callee::Method { method, .. },
                                ..
                            },
                        ..
                    } if method == "add"
                )
            })
            .count();
        assert_eq!(add_calls, 2, "{:#?}", twice.blocks);
    }

    #[test]
    fn infers_member_call_type_from_overloaded_variadic_arity() {
        let program = parse_inline(
            r#"
            class Exec {}

            class Runner {

                def exec(sql Str) Exec = Exec()
                def exec(sql Str, first Any, rest [Any] vararg) Result[Int, Str] = Ok(1)
}


            def main(r Runner) Unit {
                staged = r.exec("update users")
                direct = r.exec("update users", true, 1)
            }
            "#,
        );

        let lowered = lower_program(&program);
        assert!(lowered.diagnostics.is_empty(), "{:#?}", lowered.diagnostics);
        let ir = lowered.program.expect("ir program");
        let main = ir.entry.and_then(|id| ir.function(id)).expect("main");
        let staged = main
            .locals
            .iter()
            .find(|local| local.name == "staged")
            .expect("staged local");
        let direct = main
            .locals
            .iter()
            .find(|local| local.name == "direct")
            .expect("direct local");

        assert_eq!(staged.ty, ir::Type::named("Exec"));
        assert_eq!(
            direct.ty,
            ir::Type::Named {
                name: "Result".to_string(),
                args: vec![ir::Type::named("Int"), ir::Type::named("Str")],
            }
        );
    }

    #[test]
    fn infers_generic_method_type_from_trailing_lambda_return() {
        let program = parse_inline(
            r#"
            interface Runner {
                def perform[T](work fn(Int) Result[T, Str]) Result[T, Str]
            }

            def run(runner Runner) Result[Unit, Str] {
                try runner.perform { (value Int) => Ok(()) }
                Ok(())
            }
            "#,
        );

        let lowered = lower_program(&program);
        assert!(lowered.diagnostics.is_empty(), "{:#?}", lowered.diagnostics);
        let ir = lowered.program.expect("ir program");
        let lambda = ir
            .functions
            .iter()
            .find(|function| matches!(function.kind, ir::FunctionKind::Lambda))
            .expect("trailing lambda");
        assert_eq!(
            lambda.return_ty,
            ir::Type::Named {
                name: "Result".to_string(),
                args: vec![ir::Type::Unit, ir::Type::named("Str")],
            }
        );
    }

    #[test]
    fn infers_builtin_vector_pipeline_result_types() {
        let program = parse_inline(
            r#"
            class Entry {
                value Int
            }

            def main(entries Vector[Entry]) Unit {
                selected = entries.filter { entry => entry.value > 0 }
                total = selected.fold(0, (sum, entry) => sum + entry.value)
                labels = selected.map { entry => entry.value.toStr() }
                first = labels.take(1)
                println(total, first)
            }
            "#,
        );

        let lowered = lower_program(&program);
        assert!(lowered.diagnostics.is_empty(), "{:#?}", lowered.diagnostics);
        let ir = lowered.program.expect("ir program");
        let main = ir.entry.and_then(|id| ir.function(id)).expect("main");
        let local_type = |name: &str| {
            main.locals
                .iter()
                .find(|local| local.name == name)
                .map(|local| local.ty.clone())
                .expect("pipeline local")
        };

        assert_eq!(
            local_type("selected"),
            ir::Type::list(ir::Type::named("Entry"))
        );
        assert_eq!(local_type("total"), ir::Type::Int);
        assert_eq!(local_type("labels"), ir::Type::list(ir::Type::Str));
        assert_eq!(local_type("first"), ir::Type::list(ir::Type::Str));
    }

    #[test]
    fn lowers_match_try_and_for_forms() {
        let program = parse_inline(
            r#"
            def main() Int {
                total Int = 0
                item = try Some(3)
                total = match item {
                    case 1 => 10
                    case _ => 20
                }
                for value <- [1, 2, 3] {
                    total += value
                }
                return total
            }
            "#,
        );

        let lowered = lower_program(&program);
        assert!(lowered.diagnostics.is_empty(), "{:#?}", lowered.diagnostics);
        let ir = lowered.program.expect("ir program");
        let main = ir.entry.and_then(|id| ir.function(id)).expect("main");
        assert!(main.blocks.len() >= 8, "{:#?}", main.blocks);
        assert!(
            main.blocks
                .iter()
                .any(|block| matches!(block.terminator.kind, ir::TerminatorKind::Branch { .. }))
        );
        assert!(main.blocks.iter().any(|block| {
            block.statements.iter().any(|stmt| match &stmt.kind {
                ir::StatementKind::Assign {
                    value:
                        ir::RValue::Call {
                            callee: ir::Callee::Method { method, .. },
                            ..
                        },
                    ..
                } => method == "isSuccess",
                _ => false,
            })
        }));
        assert!(main.blocks.iter().any(|block| {
            block.statements.iter().any(|stmt| {
                matches!(
                    stmt.kind,
                    ir::StatementKind::Assign {
                        value: ir::RValue::Call {
                            callee: ir::Callee::Intrinsic(ir::Intrinsic::IterHasNext),
                            ..
                        },
                        ..
                    }
                )
            })
        }));
    }

    #[test]
    fn lowers_local_functions_lambdas_and_shape_updates() {
        let program = parse_inline(
            r#"
            shape Amount {
                amount Int
                description Str
            }

            def main() Int {
                base = 10
                inc fn(Int) Int = value => value + 1
                plus = (value Int) => value + base

                def add(value Int) Int = plus(value)

                current = Amount(1, "a")
                updated = current with { amount: add(inc(1)) }
                return updated.amount
            }
            "#,
        );

        let lowered = lower_program(&program);
        assert!(lowered.diagnostics.is_empty(), "{:#?}", lowered.diagnostics);
        let ir = lowered.program.expect("ir program");
        assert!(
            ir.functions.len() >= 4,
            "expected nested functions for lambdas/local defs, got {:#?}",
            ir.functions
        );
        let main = ir.entry.and_then(|id| ir.function(id)).expect("main");
        assert!(main.blocks.iter().any(|block| {
            block.statements.iter().any(|stmt| {
                matches!(
                    stmt.kind,
                    ir::StatementKind::Assign {
                        value: ir::RValue::Closure { .. },
                        ..
                    }
                )
            })
        }));
        assert!(main.blocks.iter().any(|block| {
            block.statements.iter().any(|stmt| {
                matches!(
                    stmt.kind,
                    ir::StatementKind::Assign {
                        value: ir::RValue::RecordUpdate { .. },
                        ..
                    }
                )
            })
        }));
    }

    #[test]
    fn lowers_transitive_lambda_captures_and_enclosing_method_calls() {
        let program = parse_inline(
            r#"
            class Tracker {
                def min(left Int, right Int) Int = if left <= right { left } else { right }

                def matching(position Int, groups [[Int]]) [[Int]] =
                    groups.map(group => group.filter(value => value == position))

                def cappedTotal(values [Int], limit Int) Int =
                    values.fold(0, (acc, value) => acc + min(value, limit))
            }
            "#,
        );

        let lowered = lower_program(&program);
        assert!(lowered.diagnostics.is_empty(), "{:#?}", lowered.diagnostics);
        let ir = lowered.program.expect("ir program");
        assert!(
            ir.functions
                .iter()
                .filter(|function| matches!(function.kind, ir::FunctionKind::Lambda))
                .count()
                >= 3
        );
    }

    #[test]
    fn lowers_map_literal_without_binary_operator_errors() {
        let program = parse_inline(
            r#"
            def main() Unit {
                entries = ["a": 1, "bbb": 2]
                OS.println(entries)
            }
            "#,
        );

        let lowered = lower_program(&program);
        assert!(lowered.diagnostics.is_empty(), "{:#?}", lowered.diagnostics);
        let ir = lowered.program.expect("ir program");
        let main = ir.entry.and_then(|id| ir.function(id)).expect("main");
        assert!(main.blocks.iter().any(|block| {
            block.statements.iter().any(|stmt| {
                matches!(
                    stmt.kind,
                    ir::StatementKind::Assign {
                        value: ir::RValue::Tuple(_),
                        ..
                    }
                )
            })
        }));
    }

    #[test]
    fn retains_core_bodies_by_stable_function_id() {
        let program = parse_inline(
            r#"
            def choose(flag Bool) Int = if flag {
                1
            } else {
                2
            }
            "#,
        );

        let lowered = lower_program(&program);
        assert!(lowered.diagnostics.is_empty(), "{:#?}", lowered.diagnostics);
        let ir = lowered.program.expect("ir program");
        let choose = ir
            .functions
            .iter()
            .find(|function| function.name == "choose")
            .expect("choose function");
        assert!(matches!(
            lowered.core_bodies.get(&choose.id),
            Some(core::CallableBody::Expr(core::Expr::If { .. }))
        ));
    }

    #[test]
    fn retains_resolved_call_metadata_for_source_backends() {
        let program = parse_inline(
            r#"
            def add(left Int, right Int) Int = left + right
            def main() Int = add(1, 2)
            "#,
        );

        let lowered = lower_program(&program);
        assert!(lowered.diagnostics.is_empty(), "{:#?}", lowered.diagnostics);
        let ir = lowered.program.expect("ir program");
        let add = ir
            .functions
            .iter()
            .find(|function| function.name == "add")
            .expect("add function");
        let main = ir
            .functions
            .iter()
            .find(|function| function.name == "main")
            .expect("main function");
        let call = ir
            .source_calls
            .iter()
            .find(|call| call.function == main.id)
            .expect("resolved main call");
        assert!(matches!(call.callee, ir::Callee::Direct(id) if id == add.id));
        assert_eq!(call.ordered_arg_spans.len(), 2);
        assert_eq!(call.param_specs.len(), 2);
        assert!(
            call.param_specs.iter().all(|spec| {
                spec.as_ref().is_some_and(|spec| {
                    spec.ty == ir::Type::named("Int") && !spec.lazy && !spec.variadic
                })
            }),
            "{:#?}",
            call.param_specs
        );
        let call_expr = ir
            .source_exprs
            .iter()
            .find(|expr| expr.function == main.id && expr.span == call.span)
            .expect("checked source expression");
        assert_eq!(call_expr.ty, ir::Type::named("Int"));
    }
}
