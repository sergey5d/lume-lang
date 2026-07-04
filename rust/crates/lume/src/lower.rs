use std::collections::HashMap;

use crate::{
    ast::{self, BinaryOp as AstBinaryOp, ImplBlock, ImplTargetKind, Item, TypeDecl, TypeMember},
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
    pub diagnostics: Vec<Diagnostic>,
}

pub fn lower_program(program: &ast::Program) -> LowerResult {
    let mut lowerer = Lowerer::new(program);
    let lowered = lowerer.lower();
    LowerResult {
        program: Some(lowered),
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
    type_ids: HashMap<(String, ast::TypeKind), ir::TypeId>,
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
        Self {
            source,
            diagnostics: Vec::new(),
            program: ir::Program::new(source.module.as_ref().map(|module| module.name.clone())),
            type_ids: HashMap::new(),
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
        self.lower_top_level_functions();
        self.lower_methods();
        self.lower_field_initializers();
        self.lower_global_initializers();
        if let Some(main) = self.function_ids.get("main").copied() {
            self.program.set_entry(main);
        }
        std::mem::take(&mut self.program)
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
                    ty.with_bounds = decl.with_bounds.iter().map(lower_type_ref).collect();
                    ty.span = Some(decl.span);
                    let id = self.program.add_type(ty);
                    self.type_ids.insert(key, id);
                }
                Item::Function(function) => {
                    if self.function_ids.contains_key(&function.name) {
                        continue;
                    }
                    let id = self.declare_function(
                        &function.name,
                        function.visibility,
                        &function.annotations,
                        &function.type_params,
                        function.return_type.as_ref(),
                        ir::FunctionKind::TopLevel,
                        &function.params,
                        None,
                        function.span,
                    );
                    self.function_ids.insert(function.name.clone(), id);
                    self.function_work.push(FunctionWork {
                        id,
                        decl: desugar::desugar_function_decl(function),
                    });
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
                                .map(lower_type_ref)
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

    fn define_items(&mut self) {
        let items = self.source.items.clone();
        for item in items {
            match item {
                Item::Type(decl) => self.define_type_decl(&decl),
                Item::Impl(block) => self.define_impl_block(&block),
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
                        .map(lower_type_ref)
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
                    self.method_work.push(MethodWork {
                        id,
                        decl: desugar::desugar_method_decl(method),
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
                                .map(lower_type_ref)
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
                        name: case.name.clone(),
                        fields: case_fields,
                        span: Some(case.span),
                    });
                }
            }
        }

        let field_init = (!field_init_stmts.is_empty()).then(|| {
            let (id, this_local) = self.declare_field_init_function(type_id, &decl.name, decl.span);
            self.field_init_work.push(FieldInitWork {
                id,
                this_local,
                body: Block {
                    statements: field_init_stmts,
                    span: decl.span,
                },
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

    fn define_impl_block(&mut self, block: &ImplBlock) {
        let Some(target_name) = named_type_name(&block.target) else {
            self.add_error(
                "lower_invariant",
                "impl target should be resolved to a named type before lowering",
                block.span,
            );
            return;
        };
        let Some(type_id) = self.impl_target_type_id(target_name, block.target_kind) else {
            self.add_error(
                "lower_invariant",
                format!(
                    "impl target '{}' should be declared before lowering methods",
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
            self.method_work.push(MethodWork {
                id,
                decl: desugar::desugar_method_decl(method),
                this_local,
            });
        }

        if let Some(ty) = self.program.types.get_mut(type_id.0) {
            ty.methods.extend(method_ids);
        }
    }

    fn impl_target_type_id(
        &self,
        target_name: &str,
        target_kind: ImplTargetKind,
    ) -> Option<ir::TypeId> {
        match target_kind {
            ImplTargetKind::Single => self
                .type_ids
                .get(&(target_name.to_string(), ast::TypeKind::Single))
                .copied(),
            ImplTargetKind::Instance => self.type_ids.iter().find_map(|((name, kind), id)| {
                (name == target_name && *kind != ast::TypeKind::Single).then_some(*id)
            }),
        }
    }

    fn declare_function(
        &mut self,
        name: &str,
        visibility: ast::Visibility,
        annotations: &[ast::Annotation],
        type_params: &[ast::TypeParam],
        return_type: Option<&TypeRef>,
        kind: ir::FunctionKind,
        params: &[core::Param],
        this_local: Option<(String, ir::Type)>,
        span: Span,
    ) -> ir::FunctionId {
        let type_param_names = type_params
            .iter()
            .map(|param| param.name.clone())
            .collect::<Vec<_>>();
        let mut function = ir::Function::new(
            name.to_string(),
            kind,
            return_type
                .map(|ty| lower_type_ref_with_type_params(ty, &type_param_names))
                .unwrap_or(ir::Type::Unknown),
        );
        function.annotations = lower_annotations(annotations);
        function.visibility = visibility;
        function.type_params = type_param_names.clone();
        function.reified_type_params = type_params
            .iter()
            .filter(|param| param.reified)
            .map(|param| param.name.clone())
            .collect();
        function.span = Some(span);
        if let Some((this_name, this_ty)) = this_local {
            function.add_local(this_name, this_ty, false, ir::LocalKind::Capture);
        }
        for (index, param) in params.iter().enumerate() {
            let runtime_ty = param
                .ty
                .as_ref()
                .map(|ty| lower_type_ref_with_type_params(ty, &type_param_names))
                .unwrap_or(ir::Type::Unknown);
            function.add_param(param.name.clone(), runtime_ty);
            function.set_param_default(
                index,
                param
                    .initializer
                    .as_ref()
                    .and_then(|initializer| lower_field_initializer_constant(Some(initializer))),
            );
            function.set_param_variadic(index, param.variadic);
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
        let id = self.declare_function(
            &method.name,
            method.visibility,
            &method.annotations,
            &method.type_params,
            method.return_type.as_ref(),
            ir::FunctionKind::Method { owner },
            &method.params,
            Some((String::from("this"), ir::Type::named(owner_name))),
            method.span,
        );
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
            ast::Visibility::Hidden,
            &[],
            &[],
            Some(&TypeRef::Named {
                name: "Unit".to_string(),
                args: Vec::new(),
                span,
            }),
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
                job.id,
                &self.global_ids,
                &self.function_ids,
                &self.case_fields,
                &mut self.diagnostics,
            );
            for (index, param) in job.decl.params.iter().enumerate() {
                if let Some(local_id) = lowerer.function().params.get(index).copied() {
                    if param.name != "_" {
                        lowerer.bind_existing(&param.name, local_id);
                    }
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
                job.id,
                &self.global_ids,
                &self.function_ids,
                &self.case_fields,
                &mut self.diagnostics,
            );
            lowerer.bind_existing("this", job.this_local);
            for (index, param) in job.decl.params.iter().enumerate() {
                if let Some(local_id) = lowerer.function().params.get(index).copied() {
                    if param.name != "_" {
                        lowerer.bind_existing(&param.name, local_id);
                    }
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
        init.visibility = ast::Visibility::Hidden;
        let init_id = self.program.add_function(init);
        self.program.set_global_init(init_id);

        let jobs = self.global_inits.clone();
        let mut lowerer = FunctionLowerer::new(
            &mut self.program,
            init_id,
            &self.global_ids,
            &self.function_ids,
            &self.case_fields,
            &mut self.diagnostics,
        );
        for job in jobs {
            let value = lowerer.lower_expr(&job.expr);
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
                job.id,
                &self.global_ids,
                &self.function_ids,
                &self.case_fields,
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
    function_id: ir::FunctionId,
    diagnostics: &'a mut Vec<Diagnostic>,
    globals: &'a HashMap<String, ir::GlobalId>,
    functions: &'a HashMap<String, ir::FunctionId>,
    case_fields: &'a HashMap<String, Vec<String>>,
    scopes: Vec<HashMap<String, ir::LocalId>>,
    capture_sources: HashMap<String, CaptureSource>,
    capture_locals: HashMap<String, ir::LocalId>,
    closure_captures: Vec<ir::Operand>,
    this_local: Option<ir::LocalId>,
    loop_exits: Vec<ir::BlockId>,
    loop_continues: Vec<ir::BlockId>,
    current_block: Option<ir::BlockId>,
}

#[derive(Debug, Clone)]
struct CaptureSource {
    operand: ir::Operand,
    ty: ir::Type,
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
        function_id: ir::FunctionId,
        globals: &'a HashMap<String, ir::GlobalId>,
        functions: &'a HashMap<String, ir::FunctionId>,
        case_fields: &'a HashMap<String, Vec<String>>,
        diagnostics: &'a mut Vec<Diagnostic>,
    ) -> Self {
        let entry = program
            .function(function_id)
            .map(|function| function.entry)
            .unwrap_or(ir::BlockId(0));
        let mut this = Self {
            program,
            function_id,
            diagnostics,
            globals,
            functions,
            case_fields,
            scopes: vec![HashMap::new()],
            capture_sources: HashMap::new(),
            capture_locals: HashMap::new(),
            closure_captures: Vec::new(),
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
        let result = match body {
            CallableBody::Expr(expr) => Some(self.lower_expr(expr)),
            CallableBody::Block(block) => self.lower_block_value(block),
        };
        if let Some(block) = self.current_block_mut() {
            block.set_terminator(ir::Terminator {
                span: Some(span),
                kind: ir::TerminatorKind::Return(result),
            });
        }
    }

    fn lower_block_value(&mut self, block: &Block) -> Option<ir::Operand> {
        self.push_scope();
        let mut tail = None;
        for (index, stmt) in block.statements.iter().enumerate() {
            let is_last = index + 1 == block.statements.len();
            if is_last {
                match stmt {
                    Stmt::Expr(expr_stmt) => {
                        tail = Some(self.lower_expr(&expr_stmt.expr));
                        break;
                    }
                    Stmt::If(if_stmt) => {
                        if let Some(value) = self.lower_if_stmt_tail_value(if_stmt) {
                            tail = Some(value);
                            break;
                        }
                    }
                    Stmt::Match(match_stmt) => {
                        tail = Some(self.lower_match_expr(
                            match_stmt.partial,
                            &match_stmt.value,
                            &match_stmt.cases,
                            match_stmt.span,
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
                    let ty = local.ty.as_ref().map(lower_type_ref).unwrap_or_else(|| {
                        if destructure_single_value {
                            ir::Type::Unknown
                        } else {
                            binding
                                .values
                                .get(index)
                                .map(|expr| inferred_storage_type(self.infer_expr_type(expr)))
                                .unwrap_or(ir::Type::Unknown)
                        }
                    });
                    let local_id = self.add_local(
                        local.name.clone(),
                        ty,
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
                        binding.values.get(index).map(|expr| self.lower_expr(expr))
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
                    let Some(target) = self.lower_place(target_expr) else {
                        continue;
                    };
                    let value =
                        if matches!(assignment.operator, AssignOp::Assign | AssignOp::Reassign) {
                            ir::RValue::Use(self.lower_expr(value_expr))
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
                let value = ret.value.as_ref().map(|expr| self.lower_expr(expr));
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
                        .map(lower_type_ref)
                        .unwrap_or(ir::Type::Unknown)
                })
                .collect(),
            ret: Box::new(
                function
                    .return_type
                    .as_ref()
                    .map(lower_type_ref)
                    .unwrap_or(ir::Type::Unknown),
            ),
        };
        let local_id = self.add_local(function.name.clone(), ty, false, ir::LocalKind::Binding);
        self.current_scope().insert(function.name.clone(), local_id);

        let closure = self.lower_nested_function_decl(function);
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
                .map(lower_type_ref)
                .unwrap_or(ir::Type::Unknown),
        );
        nested.span = Some(function.span);
        for (index, param) in function.params.iter().enumerate() {
            let runtime_ty = param
                .ty
                .as_ref()
                .map(lower_type_ref)
                .unwrap_or(ir::Type::Unknown);
            nested.add_param(param.name.clone(), runtime_ty);
            nested.set_param_variadic(index, param.variadic);
        }
        let function_id = self.program.add_function(nested);
        let capture_sources = self.visible_capture_sources(Some(&function.name));
        let captures = {
            let mut lowerer = FunctionLowerer::new(
                self.program,
                function_id,
                self.globals,
                self.functions,
                self.case_fields,
                self.diagnostics,
            )
            .with_capture_sources(capture_sources);
            for (index, param) in function.params.iter().enumerate() {
                if let Some(local_id) = lowerer.function().params.get(index).copied() {
                    if param.name != "_" {
                        lowerer.bind_existing(&param.name, local_id);
                    }
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
    ) -> ir::RValue {
        let nested_name = format!(
            "lambda${}${}",
            self.function_id.0,
            self.function().blocks.len()
        );
        let mut nested =
            ir::Function::new(nested_name, ir::FunctionKind::Lambda, ir::Type::Unknown);
        nested.span = Some(span);
        for (index, param) in params.iter().enumerate() {
            nested.add_param(
                lower_lambda_param_name(param, index),
                lower_lambda_param_type(param),
            );
        }
        let function_id = self.program.add_function(nested);
        let capture_sources = self.visible_capture_sources(None);
        let captures = {
            let mut lowerer = FunctionLowerer::new(
                self.program,
                function_id,
                self.globals,
                self.functions,
                self.case_fields,
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

    fn lower_lifted_segment_closure(
        &mut self,
        param: &str,
        param_ty: ir::Type,
        body: &Expr,
        span: Span,
    ) -> ir::RValue {
        let nested_name = format!(
            "lifted${}${}",
            self.function_id.0,
            self.function().blocks.len()
        );
        let mut nested =
            ir::Function::new(nested_name, ir::FunctionKind::Lambda, ir::Type::Unknown);
        nested.span = Some(span);
        nested.add_param(param.to_string(), param_ty);
        let function_id = self.program.add_function(nested);
        let capture_sources = self.visible_capture_sources(None);
        let captures = {
            let mut lowerer = FunctionLowerer::new(
                self.program,
                function_id,
                self.globals,
                self.functions,
                self.case_fields,
                self.diagnostics,
            )
            .with_capture_sources(capture_sources);
            if let Some(local_id) = lowerer.function().params.first().copied() {
                if param != "_" {
                    lowerer.bind_existing(param, local_id);
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
            return_type.map(lower_type_ref).unwrap_or(ir::Type::Unknown),
        );
        nested.span = Some(span);
        for (index, param) in params.iter().enumerate() {
            let runtime_ty = param
                .ty
                .as_ref()
                .map(lower_type_ref)
                .unwrap_or(ir::Type::Unknown);
            nested.add_param(param.name.clone(), runtime_ty);
            nested.set_param_variadic(index, param.variadic);
        }
        let function_id = self.program.add_function(nested);
        let capture_sources = self.visible_capture_sources(None);
        let captures = {
            let mut lowerer = FunctionLowerer::new(
                self.program,
                function_id,
                self.globals,
                self.functions,
                self.case_fields,
                self.diagnostics,
            )
            .with_capture_sources(capture_sources);
            for (index, param) in params.iter().enumerate() {
                if let Some(local_id) = lowerer.function().params.get(index).copied() {
                    if param.name != "_" {
                        lowerer.bind_existing(&param.name, local_id);
                    }
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

    fn lower_anonymous_interface_rvalue(
        &mut self,
        interfaces: &[TypeRef],
        methods: &[MethodDecl],
    ) -> ir::RValue {
        let methods = methods
            .iter()
            .map(|method| {
                let ir::RValue::Closure { function, captures } = self.lower_callable_closure(
                    &method.name,
                    &method.params,
                    method.return_type.as_ref(),
                    method.body.as_ref(),
                    method.span,
                ) else {
                    unreachable!("anonymous interface methods lower to closures")
                };
                ir::AnonymousInterfaceMethod {
                    name: method.name.clone(),
                    function,
                    captures,
                }
            })
            .collect();
        ir::RValue::AnonymousInterface {
            interfaces: interfaces.iter().map(lower_type_ref).collect(),
            methods,
        }
    }

    fn visible_capture_sources(
        &self,
        excluded_name: Option<&str>,
    ) -> HashMap<String, CaptureSource> {
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
                sources.insert(
                    name.clone(),
                    CaptureSource {
                        operand: ir::Operand::Copy(Box::new(ir::Place::Local(*local))),
                        ty,
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
        self.lower_block_statements(&stmt.then_block);
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
        match stmt.kind {
            core::PatternBindingKind::Let => {
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
            core::PatternBindingKind::Expect => {
                if !stmt.clauses.is_empty() {
                    let failure_block = self.add_block();
                    let continue_block = self.add_block();

                    self.lower_refutable_clause_chain(&stmt.clauses, continue_block, failure_block);

                    self.current_block = Some(failure_block);
                    self.emit_panic("expect pattern did not match", stmt.span);
                    self.terminate(ir::Terminator {
                        span: Some(stmt.span),
                        kind: ir::TerminatorKind::Unreachable,
                    });

                    self.current_block = Some(continue_block);
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
                self.apply_pending_bindings(plan.bindings);
                if self.current_block.is_some() {
                    self.terminate(ir::Terminator::goto(continue_block));
                }

                self.current_block = Some(failure_block);
                self.emit_panic("expect pattern did not match", stmt.span);
                self.terminate(ir::Terminator {
                    span: Some(stmt.span),
                    kind: ir::TerminatorKind::Unreachable,
                });

                self.current_block = Some(continue_block);
            }
        }
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
        self.apply_pending_bindings(plan.bindings);
        if self.current_block.is_some() {
            self.terminate(ir::Terminator::goto(continue_block));
        }

        self.current_block = Some(failure_block);
        let value = self
            .lower_block_value(&stmt.else_block)
            .unwrap_or(ir::Operand::Const(ir::Constant::Unit));
        if self.current_block.is_some() {
            self.terminate(ir::Terminator::ret(Some(value)));
        }

        self.current_block = Some(continue_block);
    }

    fn lower_let_else_clauses(&mut self, stmt: &core::LetElseStmt) {
        let failure_block = self.add_block();
        let continue_block = self.add_block();

        self.lower_refutable_clause_chain(&stmt.clauses, continue_block, failure_block);

        self.current_block = Some(failure_block);
        let value = self
            .lower_block_value(&stmt.else_block)
            .unwrap_or(ir::Operand::Const(ir::Constant::Unit));
        if self.current_block.is_some() {
            self.terminate(ir::Terminator::ret(Some(value)));
        }

        self.current_block = Some(continue_block);
    }

    fn lower_refutable_clause_chain(
        &mut self,
        clauses: &[core::RefutableClause],
        success_target: ir::BlockId,
        failure_target: ir::BlockId,
    ) {
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
            self.apply_pending_bindings(plan.bindings);
            if index + 1 == clauses.len() {
                break;
            }
        }
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
                callee: ir::Callee::Method {
                    receiver: ir::Operand::Copy(Box::new(ir::Place::Local(source_local))),
                    method: "orPanic".to_string(),
                },
                args: Vec::new(),
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

    fn lower_if_stmt_tail_value(&mut self, stmt: &core::IfStmt) -> Option<ir::Operand> {
        let condition = stmt.condition.clone()?;
        if !stmt.condition_clauses.is_empty()
            || !stmt.pattern_clauses.is_empty()
            || !stmt.bindings.is_empty()
            || stmt.binding_value.is_some()
        {
            return None;
        }
        let else_branch = lower_if_stmt_else_expr(stmt.else_branch.as_ref()?)?;
        let expr = Expr::If {
            condition: Box::new(condition),
            then_block: stmt.then_block.clone(),
            else_branch: Box::new(else_branch),
            span: stmt.span,
        };
        Some(self.lower_expr(&expr))
    }

    fn lower_while_stmt(&mut self, stmt: &core::WhileStmt) {
        let cond_block = self.add_block();
        let body_block = self.add_block();
        let exit_block = self.add_block();

        self.terminate(ir::Terminator::goto(cond_block));

        self.current_block = Some(cond_block);
        let cond = self.lower_expr(&stmt.condition);
        self.terminate(ir::Terminator {
            span: Some(stmt.span),
            kind: ir::TerminatorKind::Branch {
                condition: cond,
                then_block: body_block,
                else_block: exit_block,
            },
        });

        self.loop_exits.push(exit_block);
        self.loop_continues.push(cond_block);
        self.current_block = Some(body_block);
        self.lower_block_statements(&stmt.body);
        if self.current_block.is_some() {
            self.terminate(ir::Terminator::goto(cond_block));
        }
        self.loop_exits.pop();
        self.loop_continues.pop();

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
                        .map(lower_type_ref)
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
                .map(lower_type_ref)
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
            .map(lower_type_ref)
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
            .map(lower_type_ref)
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

        for (index, case) in stmt.cases.iter().enumerate() {
            let body_block = self.add_block();
            let fail_block = if index + 1 == stmt.cases.len() {
                join_block
            } else {
                self.add_block()
            };
            self.lower_match_case(
                scrutinee.clone(),
                case,
                body_block,
                fail_block,
                None,
                false,
                join_block,
                stmt.span,
            );
            self.current_block = Some(fail_block);
        }

        if self.current_block.is_some() && self.current_block != Some(join_block) {
            self.terminate(ir::Terminator::goto(join_block));
        }
        self.current_block = Some(join_block);
    }

    fn lower_match_expr(
        &mut self,
        partial: bool,
        value: &Expr,
        cases: &[core::MatchCase],
        span: Span,
    ) -> ir::Operand {
        let scrutinee = self.lower_expr(value);
        let result = self.add_temp(if partial {
            ir::Type::option(ir::Type::Unknown)
        } else {
            ir::Type::Unknown
        });
        let join_block = self.add_block();
        self.current_block = self.current_block.or(Some(self.function().entry));

        for case in cases {
            let body_block = self.add_block();
            let fail_block = self.add_block();
            self.lower_match_case(
                scrutinee.clone(),
                case,
                body_block,
                fail_block,
                Some(result),
                partial,
                join_block,
                span,
            );
            self.current_block = Some(fail_block);
        }

        if let Some(block) = self.current_block_mut() {
            let default_value = if partial {
                ir::RValue::NamedValue {
                    path: vec!["None".to_string()],
                }
            } else {
                ir::RValue::Use(ir::Operand::Const(ir::Constant::Unit))
            };
            block.push(ir::Statement {
                span: Some(span),
                kind: ir::StatementKind::Assign {
                    target: ir::Place::Local(result),
                    value: default_value,
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
        fail_block: ir::BlockId,
        result_target: Option<ir::LocalId>,
        partial: bool,
        join_block: ir::BlockId,
        span: Span,
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
                    else_block: fail_block,
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
                    else_block: fail_block,
                },
            });
            self.pop_scope();
        } else {
            self.terminate(ir::Terminator {
                span: Some(case.span),
                kind: ir::TerminatorKind::Branch {
                    condition,
                    then_block: body_block,
                    else_block: fail_block,
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
                        .lower_block_value(block)
                        .unwrap_or(ir::Operand::Const(ir::Constant::Unit));
                    self.assign_match_result(target, value, partial, span);
                } else {
                    self.lower_block_statements(block);
                }
            }
            MatchCaseBody::Expr(expr) => {
                let value = self.lower_expr(expr);
                if let Some(target) = result_target {
                    self.assign_match_result(target, value, partial, span);
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

    fn assign_match_result(
        &mut self,
        target: ir::LocalId,
        value: ir::Operand,
        partial: bool,
        span: Span,
    ) {
        let rvalue = if partial {
            ir::RValue::Call {
                callee: ir::Callee::Named {
                    path: vec!["Some".to_string()],
                },
                args: vec![value],
                structural: false,
            }
        } else {
            ir::RValue::Use(value)
        };
        self.push_statement(ir::Statement {
            span: Some(span),
            kind: ir::StatementKind::Assign {
                target: ir::Place::Local(target),
                value: rvalue,
            },
        });
    }

    fn lower_pattern_plan(&mut self, scrutinee: ir::Operand, pattern: &Pattern) -> PatternPlan {
        match pattern {
            Pattern::Wildcard { .. } => PatternPlan::always_true(),
            Pattern::Extract { inner, span } => {
                let base_condition = self.emit_temp_from_rvalue(
                    ir::RValue::Call {
                        callee: ir::Callee::Intrinsic(ir::Intrinsic::ExtractSuccessIsSet),
                        args: vec![scrutinee.clone()],
                        structural: false,
                    },
                    ir::Type::Bool,
                    Some(*span),
                );
                let payload = self.emit_temp_from_rvalue(
                    ir::RValue::Call {
                        callee: ir::Callee::Intrinsic(ir::Intrinsic::ExtractSuccessValue),
                        args: vec![scrutinee],
                        structural: false,
                    },
                    ir::Type::Unknown,
                    Some(*span),
                );
                let inner_plan = self.lower_pattern_plan(payload, inner);
                PatternPlan {
                    condition: self
                        .combine_conditions(vec![base_condition, inner_plan.condition], *span),
                    bindings: inner_plan.bindings,
                }
            }
            Pattern::Binding { name, .. } => {
                if name == "_" {
                    PatternPlan::always_true()
                } else {
                    PatternPlan {
                        condition: self.bool_const(true),
                        bindings: vec![PendingBinding {
                            name: name.clone(),
                            ty: ir::Type::Unknown,
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
                let ty = lower_type_ref(target);
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
            Pattern::Constructor { path, args, span } => {
                let Some(kind) = self.lookup_constructor_pattern_kind(path, args.len()) else {
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
                };
                let mut conditions = vec![base_condition];
                let mut bindings = Vec::new();
                let field_names = match kind {
                    ConstructorPatternKind::EnumCase { field_names, .. }
                    | ConstructorPatternKind::TypeDestructure { field_names, .. } => field_names,
                };
                for (index, arg) in args.iter().enumerate() {
                    let field_name = field_names
                        .get(index)
                        .cloned()
                        .unwrap_or_else(|| format!("_{}", index + 1));
                    let field = self.emit_temp_from_rvalue(
                        ir::RValue::Call {
                            callee: ir::Callee::Intrinsic(ir::Intrinsic::VariantField(field_name)),
                            args: vec![scrutinee.clone()],
                            structural: false,
                        },
                        ir::Type::Unknown,
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

    fn lower_pattern_literal_expr(&mut self, expr: &ast::Expr) -> ir::Operand {
        let expr = desugar::desugar_expr(expr);
        self.lower_expr(&expr)
    }

    fn lookup_constructor_pattern_kind(
        &self,
        path: &[String],
        arity: usize,
    ) -> Option<ConstructorPatternKind> {
        if let Some(field_names) = self.lookup_case_fields(path, arity) {
            return Some(ConstructorPatternKind::EnumCase {
                case_name: path.last().cloned().unwrap_or_default(),
                field_names,
            });
        }
        if let Some((ty, field_names)) = self.lookup_destructured_type_fields(path, arity) {
            return Some(ConstructorPatternKind::TypeDestructure { ty, field_names });
        }
        None
    }

    fn lookup_case_fields(&self, path: &[String], arity: usize) -> Option<Vec<String>> {
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
                if let Some(fields) = enum_case_pattern_fields(case, arity) {
                    return Some(fields);
                }
            }
        }
        match case_name.as_str() {
            "Some" | "Ok" | "Left" | "Right" if arity == 1 => {
                return Some(vec!["value".to_string()]);
            }
            "Err" if arity == 1 => {
                return Some(vec!["error".to_string()]);
            }
            "None" if arity == 0 => return Some(Vec::new()),
            _ => {}
        }

        let ast_matches = self
            .program
            .types
            .iter()
            .filter(|ty| ty.kind == ast::TypeKind::Enum)
            .flat_map(|ty| ty.enum_cases.iter())
            .filter(|case| case.name == *case_name)
            .filter_map(|case| enum_case_pattern_fields(case, arity))
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
        let type_name = match path {
            [name] => name,
            [_, name] => name,
            _ => return None,
        };
        let ty = self.program.types.iter().find(|ty| {
            ty.name == *type_name && matches!(ty.kind, ast::TypeKind::Class | ast::TypeKind::Record)
        })?;
        let visible_fields = ty
            .fields
            .iter()
            .filter(|field| field.visibility != ast::Visibility::Hidden)
            .collect::<Vec<_>>();
        if visible_fields.len() != arity {
            return None;
        }
        Some((
            ir::Type::named(type_name.clone()),
            visible_fields
                .iter()
                .map(|field| field.name.clone())
                .collect(),
        ))
    }

    fn apply_pending_bindings(&mut self, bindings: Vec<PendingBinding>) {
        for binding in bindings {
            let local_id = self.add_local(
                binding.name.clone(),
                binding.ty,
                false,
                ir::LocalKind::Binding,
            );
            self.current_scope().insert(binding.name.clone(), local_id);
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
        }
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

    fn lower_expr(&mut self, expr: &Expr) -> ir::Operand {
        if self.current_block.is_none() {
            return ir::Operand::Const(ir::Constant::Unit);
        }
        match expr {
            Expr::Identifier { name, span } => self
                .lookup_scoped_or_captured_value(name)
                .unwrap_or_else(|| {
                    if let Some(place) = self.lookup_implicit_field_place(name) {
                        return ir::Operand::Copy(Box::new(place));
                    }
                    if let Some(place) = self.lookup_global_place(name) {
                        return ir::Operand::Copy(Box::new(place));
                    }
                    let path = vec![name.clone()];
                    if is_named_runtime_value_path(self.program, &path) {
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
                }),
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
                condition,
                then_block,
                else_branch,
                span,
            } => self.lower_if_expr(condition, then_block, else_branch, *span),
            Expr::Try { value, span } => self.lower_try_expr(value, *span),
            Expr::Block { body, .. } => self.lower_block_expr(body),
            Expr::Match {
                partial,
                value,
                cases,
                span,
            } => self.lower_match_expr(*partial, value, cases, *span),
            Expr::ForYield {
                bindings,
                yield_body,
                span,
            } => self.lower_for_yield_expr(bindings, yield_body, *span),
            Expr::LiftedChain {
                base,
                segments,
                span,
            } => self.lower_lifted_chain_expr(base, segments, *span),
            Expr::ListLiteral { .. }
            | Expr::TupleLiteral { .. }
            | Expr::RecordLiteral { .. }
            | Expr::Lift { .. }
            | Expr::AnonymousInterface { .. }
            | Expr::Unary { .. }
            | Expr::Binary { .. }
            | Expr::Call { .. }
            | Expr::Member { .. }
            | Expr::Index { .. }
            | Expr::RecordUpdate { .. }
            | Expr::Is { .. }
            | Expr::TypeOf { .. }
            | Expr::Lambda { .. } => self.lower_expr_from_rvalue(expr),
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

    fn lower_expr_from_rvalue(&mut self, expr: &Expr) -> ir::Operand {
        let ty = inferred_storage_type(self.infer_expr_type(expr));
        let rvalue = self
            .lower_rvalue(expr)
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
        let source = self.lower_expr(value);
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
                callee: ir::Callee::Method {
                    receiver: ir::Operand::Copy(Box::new(ir::Place::Local(source_local))),
                    method: "orPanic".to_string(),
                },
                args: Vec::new(),
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
        self.terminate(ir::Terminator::ret(Some(ir::Operand::Copy(Box::new(
            ir::Place::Local(source_local),
        )))));

        let join_used = self.block_has_predecessor(join_block);
        self.current_block = if join_used { Some(join_block) } else { None };
        ir::Operand::Copy(Box::new(ir::Place::Local(result)))
    }

    fn lower_lifted_chain_expr(
        &mut self,
        base: &Expr,
        segments: &[core::LiftedChainSegment],
        span: Span,
    ) -> ir::Operand {
        let mut current = self.lower_expr(base);
        let mut current_ty = self.infer_expr_type(base);

        for segment in segments {
            let Some((family, inner_ty)) = unwrap_lifted_ir_type(&current_ty) else {
                self.add_error(
                    "lower_invariant",
                    format!(
                        ".-> receiver should be lifted before lowering, got '{}'",
                        describe_ir_type(&current_ty)
                    ),
                    segment.span,
                );
                return current;
            };
            let segment_ty = self.infer_lifted_segment_type(segment, &inner_ty);
            let method = if lifted_ir_segment_flattens(self.program, &family, &segment_ty) {
                "flatMap"
            } else {
                "map"
            };
            let result_ty =
                lifted_ir_segment_result_type(self.program, &family, segment_ty.clone());
            let closure = self.lower_lifted_segment_closure(
                &segment.param,
                inner_ty,
                &segment.body,
                segment.span,
            );
            let closure_operand =
                self.emit_temp_from_rvalue(closure, ir::Type::Unknown, Some(segment.span));
            current = self.emit_temp_from_rvalue(
                ir::RValue::Call {
                    callee: ir::Callee::Method {
                        receiver: current,
                        method: method.to_string(),
                    },
                    args: vec![closure_operand],
                    structural: false,
                },
                result_ty.clone(),
                Some(span),
            );
            current_ty = result_ty;
        }

        current
    }

    fn infer_lifted_segment_type(
        &self,
        segment: &core::LiftedChainSegment,
        inner_ty: &ir::Type,
    ) -> ir::Type {
        self.infer_expr_type_with_overrides(
            &segment.body,
            &[(segment.param.clone(), inner_ty.clone())],
        )
    }

    fn infer_expr_type(&self, expr: &Expr) -> ir::Type {
        self.infer_expr_type_with_overrides(expr, &[])
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
                let item_ty = items
                    .iter()
                    .map(|item| self.infer_expr_type_with_overrides(item, overrides))
                    .reduce(join_ir_types)
                    .unwrap_or(ir::Type::Unknown);
                ir::Type::list(item_ty)
            }
            Expr::TupleLiteral { items, .. } => ir::Type::Tuple(
                items
                    .iter()
                    .map(|item| self.infer_expr_type_with_overrides(item, overrides))
                    .collect(),
            ),
            Expr::RecordLiteral { fields, values, .. } => {
                if fields.is_empty() && !values.is_empty() {
                    return ir::Type::Tuple(
                        values
                            .iter()
                            .map(|value| self.infer_expr_type_with_overrides(value, overrides))
                            .collect(),
                    );
                }
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
                            upsert_ir_record_field(&mut out, spread_field);
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
            Expr::Member { receiver, name, .. } => {
                if name == "runtimeType" {
                    let receiver_ty = self.infer_expr_type_with_overrides(receiver, overrides);
                    return ir_value_runtime_type(receiver_ty);
                }
                let receiver_ty = self.infer_expr_type_with_overrides(receiver, overrides);
                self.infer_member_type(&receiver_ty, name)
                    .unwrap_or(ir::Type::Unknown)
            }
            Expr::Index { receiver, .. } => {
                let receiver_ty = self.infer_expr_type_with_overrides(receiver, overrides);
                index_result_ir_type(&receiver_ty)
            }
            Expr::RecordUpdate { receiver, .. } => {
                self.infer_expr_type_with_overrides(receiver, overrides)
            }
            Expr::LiftedChain { base, segments, .. } => {
                let mut current = self.infer_expr_type_with_overrides(base, overrides);
                for segment in segments {
                    let Some((family, inner)) = unwrap_lifted_ir_type(&current) else {
                        return ir::Type::Unknown;
                    };
                    let segment_ty = self.infer_expr_type_with_overrides(
                        &segment.body,
                        &[(segment.param.clone(), inner)],
                    );
                    current = lifted_ir_segment_result_type(self.program, &family, segment_ty);
                }
                current
            }
            Expr::TypeOf { ty, .. } => ir_exact_runtime_type(lower_type_ref(ty)),
            Expr::AnonymousInterface { interfaces, .. } => {
                if interfaces.len() == 1 {
                    lower_type_ref(&interfaces[0])
                } else {
                    ir::Type::Unknown
                }
            }
            Expr::If { .. }
            | Expr::Block { .. }
            | Expr::Match { .. }
            | Expr::ForYield { .. }
            | Expr::Try { .. }
            | Expr::Lift { .. }
            | Expr::Unary { .. }
            | Expr::Binary { .. }
            | Expr::Is { .. }
            | Expr::Lambda { .. }
            | Expr::Placeholder { .. } => ir::Type::Unknown,
        }
    }

    fn infer_call_type(
        &self,
        callee: &Expr,
        args: &[core::CallArg],
        style: core::CallStyle,
        overrides: &[(String, ir::Type)],
    ) -> ir::Type {
        let normalized_args = self.normalize_trailing_brace_call_args(callee, args, style);
        if let Some(ty) = self.infer_builtin_case_call_type(callee, &normalized_args, overrides) {
            return ty;
        }
        if let Some(ty) = self.infer_constructor_call_type(callee) {
            return ty;
        }
        if let Some(ty) = self.infer_member_call_type(callee, &normalized_args, overrides) {
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

    fn infer_constructor_call_type(&self, callee: &Expr) -> Option<ir::Type> {
        let path = expr_path(callee)?;
        if path.len() == 1 && declared_type_exists(self.program, &path[0]) {
            return Some(ir::Type::Named {
                name: path[0].clone(),
                args: Vec::new(),
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
                        .filter(|field| field.visibility != ast::Visibility::Hidden)
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
                return self
                    .function()
                    .locals
                    .get(local.0)
                    .map(|local| local.ty.clone());
            }
        }
        self.capture_sources
            .get(name)
            .map(|source| source.ty.clone())
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

    fn lookup_enum_case_type_by_path(&self, path: &[String]) -> Option<ir::Type> {
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

    fn infer_member_type(&self, receiver: &ir::Type, name: &str) -> Option<ir::Type> {
        match receiver {
            ir::Type::Named {
                name: type_name,
                args,
            } => {
                let Some(ty) = self.program.types.iter().find(|ty| ty.name == *type_name) else {
                    return builtin_member_type(receiver, name);
                };
                let subst = ir_type_subst(ty, args);
                if let Some(field) = ty.fields.iter().find(|field| field.name == name) {
                    return Some(substitute_ir_type(&field.ty, &subst));
                }
                if let Some(function) = self.find_method_function(ty, name) {
                    let method_ty = self.function_type_with_subst(function, &subst)?;
                    if function_type_returns_unknown(&method_ty) {
                        if let Some(fallback) = builtin_member_type(receiver, name) {
                            return Some(fallback);
                        }
                    }
                    return Some(method_ty);
                }
                builtin_member_type(receiver, name)
            }
            ir::Type::Record(fields) => fields
                .iter()
                .find(|field| field.name == name)
                .map(|field| field.ty.clone()),
            ir::Type::Unknown => Some(ir::Type::Unknown),
            _ => builtin_member_type(receiver, name),
        }
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
        let receiver_ty = self.infer_expr_type_with_overrides(receiver, overrides);
        let ir::Type::Named {
            name: type_name,
            args: type_args,
        } = &receiver_ty
        else {
            return builtin_member_type(&receiver_ty, name).and_then(|ty| match ty {
                ir::Type::Function { ret, .. } => Some(*ret),
                _ => None,
            });
        };
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
                best = Some((score, substitute_ir_type(&function.return_ty, subst)));
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
        condition: &Expr,
        then_block: &Block,
        else_branch: &ElseExprBranch,
        span: Span,
    ) -> ir::Operand {
        let temp = self.add_temp(ir::Type::Unknown);
        let then_id = self.add_block();
        let else_id = self.add_block();
        let join_id = self.add_block();

        let cond = self.lower_expr(condition);
        self.terminate(ir::Terminator {
            span: Some(span),
            kind: ir::TerminatorKind::Branch {
                condition: cond,
                then_block: then_id,
                else_block: else_id,
            },
        });

        self.current_block = Some(then_id);
        if let Some(value) = self.lower_block_value(then_block) {
            self.push_statement(ir::Statement {
                span: Some(then_block.span),
                kind: ir::StatementKind::Assign {
                    target: ir::Place::Local(temp),
                    value: ir::RValue::Use(value),
                },
            });
        }
        if self.current_block.is_some() {
            self.terminate(ir::Terminator::goto(join_id));
        }

        self.current_block = Some(else_id);
        let else_value = match else_branch {
            ElseExprBranch::If(expr) => Some(self.lower_expr(expr)),
            ElseExprBranch::Block(block) => self.lower_block_value(block),
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
        if let Some(path) = expr_path(expr) {
            if path.len() > 1 && is_named_runtime_value_path(self.program, &path) {
                return Some(ir::RValue::NamedValue { path });
            }
        }
        match expr {
            Expr::ListLiteral { items, .. } => Some(ir::RValue::List(
                items.iter().map(|item| self.lower_expr(item)).collect(),
            )),
            Expr::TupleLiteral { items, .. } => Some(ir::RValue::Tuple(
                items.iter().map(|item| self.lower_expr(item)).collect(),
            )),
            Expr::RecordLiteral { fields, values, .. } => {
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
                                    ir::RecordSpreadPart::Spread(self.lower_expr(&field.value))
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
            Expr::Lift { value, .. } => Some(ir::RValue::Lift {
                value: self.lower_expr(value),
            }),
            Expr::Unary { op, expr, .. } => Some(ir::RValue::Unary {
                op: match op {
                    ast::UnaryOp::Neg => ir::UnaryOp::Neg,
                    ast::UnaryOp::Not => ir::UnaryOp::Not,
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
                ..
            } => {
                let (candidate_callee, candidate_type_args) =
                    self.split_generic_call_callee(callee);
                let candidate_normalized_args =
                    self.normalize_trailing_brace_call_args(candidate_callee, args, *style);
                let use_generic_callee = !candidate_type_args.is_empty()
                    && (self
                        .reified_call_target(candidate_callee, &candidate_normalized_args)
                        .is_some()
                        || self.is_builtin_reified_metadata_call(candidate_callee));
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
                let ordered_args = self
                    .reorder_call_args(call_callee, &normalized_args)
                    .into_iter()
                    .cloned()
                    .collect::<Vec<_>>();
                let mut lowered_args = ordered_args
                    .iter()
                    .map(|arg| self.lower_expr(&arg.value))
                    .collect::<Vec<_>>();
                lowered_args.extend(self.reified_call_evidence_args(
                    call_callee,
                    &ordered_args,
                    &explicit_type_args,
                ));
                lowered_args.extend(
                    self.builtin_reified_metadata_evidence_args(call_callee, &explicit_type_args),
                );
                Some(ir::RValue::Call {
                    callee: self.lower_callee(call_callee),
                    args: lowered_args,
                    structural: call_uses_structural_record_arg(&normalized_args, *style),
                })
            }
            Expr::Member { receiver, name, .. } => Some(ir::RValue::Field {
                base: self.lower_expr(receiver),
                name: name.clone(),
            }),
            Expr::Index {
                receiver, index, ..
            } => Some(ir::RValue::Index {
                base: self.lower_expr(receiver),
                index: self.lower_expr(index),
            }),
            Expr::Is { left, target, .. } => Some(ir::RValue::TypeTest {
                operand: self.lower_expr(left),
                ty: lower_type_ref(target),
            }),
            Expr::TypeOf { ty, .. } => {
                if let Some(operand) = self.reified_type_param_operand(ty) {
                    Some(ir::RValue::Use(operand))
                } else {
                    Some(ir::RValue::TypeOf {
                        ty: lower_type_ref(ty),
                    })
                }
            }
            Expr::Lambda { params, body, span } => {
                Some(self.lower_lambda_rvalue(params, body, *span))
            }
            Expr::AnonymousInterface {
                interfaces,
                methods,
                ..
            } => Some(self.lower_anonymous_interface_rvalue(interfaces, methods)),
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

    fn lower_binary_rvalue(&mut self, left: &Expr, op: AstBinaryOp, right: &Expr) -> ir::RValue {
        match op {
            AstBinaryOp::Colon => {
                ir::RValue::Tuple(vec![self.lower_expr(left), self.lower_expr(right)])
            }
            _ => ir::RValue::Binary {
                op: map_binary_op(op).expect("non-special binary operator should map to IR"),
                left: self.lower_expr(left),
                right: self.lower_expr(right),
            },
        }
    }

    fn lower_callee(&mut self, callee: &Expr) -> ir::Callee {
        if let Some(path) = expr_path(callee) {
            if path.len() == 1 {
                let name = &path[0];
                if name == "new" && self.function().name == "new" {
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
                if let Some(method) = self.lower_implicit_method_callee(name) {
                    return method;
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
            return ir::Callee::Method {
                receiver: self.lower_expr(receiver),
                method: name.clone(),
            };
        }

        ir::Callee::Indirect(self.lower_expr(callee))
    }

    fn lower_implicit_method_callee(&mut self, name: &str) -> Option<ir::Callee> {
        let ir::FunctionKind::Method { owner } = self.function().kind else {
            return None;
        };
        let owner_type = self.program.types.get(owner.0)?;
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
                || self.brace_call_targets_current_constructor(callee)
                || self.brace_call_targets_enum_case(callee))
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
        if path.len() != 1 {
            return false;
        }
        self.program
            .types
            .iter()
            .find(|ty| ty.name == path[0] && ty.kind == ast::TypeKind::Class)
            .is_some_and(|ty| {
                ty.methods.iter().copied().any(|id| {
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
            && path[0] == "new"
            && self.function().name == "new"
            && matches!(self.function().kind, ir::FunctionKind::Method { .. })
    }

    fn brace_call_targets_enum_case(&self, callee: &Expr) -> bool {
        let Some(path) = expr_path(callee) else {
            return false;
        };
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
                            ast::TypeKind::Class | ast::TypeKind::Record | ast::TypeKind::Single
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
                    ("List", "from") | ("Set", "from")
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
                    self.method_param_names_for_kind(owner, ast::TypeKind::Single, member, args)
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
        let _ = has_explicit_constructor;
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
            type_args.iter().map(lower_type_ref).collect(),
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

fn lower_type_ref(reference: &TypeRef) -> ir::Type {
    match reference {
        TypeRef::Wildcard { .. } => ir::Type::Unknown,
        TypeRef::Named { name, args, .. } if name == "Never" && args.is_empty() => ir::Type::Never,
        TypeRef::Named { name, args, .. } => ir::Type::Named {
            name: name.clone(),
            args: args.iter().map(lower_type_ref).collect(),
        },
        TypeRef::Tuple { fields, .. } => ir::Type::Tuple(
            fields
                .iter()
                .map(|field| lower_type_ref(&field.ty))
                .collect(),
        ),
        TypeRef::Record { fields, .. } => ir::Type::Record(
            fields
                .iter()
                .map(|field| ir::NamedType {
                    name: field.name.clone(),
                    ty: lower_type_ref(&field.ty),
                })
                .collect(),
        ),
        TypeRef::Function { params, ret, .. } => ir::Type::Function {
            params: params.iter().map(lower_type_ref).collect(),
            ret: Box::new(lower_type_ref(ret)),
        },
    }
}

fn lower_type_ref_with_type_params(reference: &TypeRef, type_params: &[String]) -> ir::Type {
    match reference {
        TypeRef::Named { name, args, .. }
            if args.is_empty() && type_params.iter().any(|param| param == name) =>
        {
            ir::Type::TypeParam(name.clone())
        }
        TypeRef::Named { name, args, .. } if name == "Never" && args.is_empty() => ir::Type::Never,
        TypeRef::Named { name, args, .. } => ir::Type::Named {
            name: name.clone(),
            args: args
                .iter()
                .map(|arg| lower_type_ref_with_type_params(arg, type_params))
                .collect(),
        },
        TypeRef::Wildcard { .. } => ir::Type::Unknown,
        TypeRef::Tuple { fields, .. } => ir::Type::Tuple(
            fields
                .iter()
                .map(|field| lower_type_ref_with_type_params(&field.ty, type_params))
                .collect(),
        ),
        TypeRef::Record { fields, .. } => ir::Type::Record(
            fields
                .iter()
                .map(|field| ir::NamedType {
                    name: field.name.clone(),
                    ty: lower_type_ref_with_type_params(&field.ty, type_params),
                })
                .collect(),
        ),
        TypeRef::Function { params, ret, .. } => ir::Type::Function {
            params: params
                .iter()
                .map(|param| lower_type_ref_with_type_params(param, type_params))
                .collect(),
            ret: Box::new(lower_type_ref_with_type_params(ret, type_params)),
        },
    }
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

fn lower_lambda_param_type(param: &core::LambdaParam) -> ir::Type {
    if let Some(ty) = &param.ty {
        return lower_type_ref(ty);
    }
    let Some(destructure) = &param.destructure else {
        return ir::Type::Unknown;
    };
    match destructure.kind {
        DestructureKind::Tuple => ir::Type::Tuple(
            destructure
                .bindings
                .iter()
                .map(|binding| {
                    binding
                        .ty
                        .as_ref()
                        .map(lower_type_ref)
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
                        .map(lower_type_ref)
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

fn lifted_ir_segment_flattens(
    program: &ir::Program,
    family: &LiftedIrFamily,
    segment_ty: &ir::Type,
) -> bool {
    match (family, segment_ty) {
        (LiftedIrFamily::Option, ir::Type::Named { name, args })
            if name == "Option" && args.len() == 1 =>
        {
            true
        }
        (LiftedIrFamily::Result { error }, ir::Type::Named { name, args })
            if name == "Result" && args.len() == 2 =>
        {
            ir_type_assignable(program, &args[1], error)
        }
        (LiftedIrFamily::Either { left }, ir::Type::Named { name, args })
            if name == "Either" && args.len() == 2 =>
        {
            ir_type_assignable(program, &args[0], left)
        }
        _ => false,
    }
}

fn lifted_ir_segment_result_type(
    program: &ir::Program,
    family: &LiftedIrFamily,
    segment_ty: ir::Type,
) -> ir::Type {
    match (family, &segment_ty) {
        (LiftedIrFamily::Option, ir::Type::Named { name, args })
            if name == "Option" && args.len() == 1 =>
        {
            return ir::Type::option(args[0].clone());
        }
        (LiftedIrFamily::Result { error }, ir::Type::Named { name, args })
            if name == "Result"
                && args.len() == 2
                && ir_type_assignable(program, &args[1], error) =>
        {
            return ir::Type::Named {
                name: "Result".to_string(),
                args: vec![args[0].clone(), error.clone()],
            };
        }
        (LiftedIrFamily::Either { left }, ir::Type::Named { name, args })
            if name == "Either"
                && args.len() == 2
                && ir_type_assignable(program, &args[0], left) =>
        {
            return ir::Type::Named {
                name: "Either".to_string(),
                args: vec![left.clone(), args[1].clone()],
            };
        }
        _ => {}
    }
    wrap_lifted_ir_type(family, segment_ty)
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
        ir::Type::Named { name, args }
            if (name == "Array" || name == "List") && args.len() == 1 =>
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

fn builtin_member_type(receiver: &ir::Type, name: &str) -> Option<ir::Type> {
    if let Some(ty) = universal_member_type(name) {
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
        ("Option", "orPanic") => Some(ir::Type::Function {
            params: Vec::new(),
            ret: Box::new(item),
        }),
        ("Option", "isDefined") => Some(ir::Type::Function {
            params: Vec::new(),
            ret: Box::new(ir::Type::Bool),
        }),
        ("Result", "orPanic") => Some(ir::Type::Function {
            params: Vec::new(),
            ret: Box::new(item),
        }),
        ("Either", "orPanic") => {
            let right = args.get(1).cloned().unwrap_or(ir::Type::Unknown);
            Some(ir::Type::Function {
                params: Vec::new(),
                ret: Box::new(right),
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
        ("Type", "asSingle") => Some(ir::Type::Function {
            params: Vec::new(),
            ret: Box::new(ir::Type::option(ir::Type::named("SingleType"))),
        }),
        ("Type", "asAnnotation") => Some(ir::Type::Function {
            params: Vec::new(),
            ret: Box::new(ir::Type::option(ir::Type::named("AnnotationType"))),
        }),
        (
            "ClassType" | "ShapeType" | "EnumType" | "InterfaceType" | "SingleType"
            | "AnnotationType",
            "name" | "qualifiedName",
        ) => Some(ir::Type::Function {
            params: Vec::new(),
            ret: Box::new(ir::Type::option(ir::Type::Str)),
        }),
        (
            "ClassType" | "ShapeType" | "EnumType" | "InterfaceType" | "SingleType"
            | "AnnotationType",
            "kind",
        ) => Some(ir::Type::Function {
            params: Vec::new(),
            ret: Box::new(ir::Type::named("TypeKind")),
        }),
        (
            "Type" | "ClassType" | "ShapeType" | "EnumType" | "InterfaceType" | "SingleType"
            | "AnnotationType" | "Field" | "Method" | "EnumCase",
            "annotation",
        ) => Some(ir::Type::Function {
            params: Vec::new(),
            ret: Box::new(ir::Type::option(ir::Type::named("AnnotationValue"))),
        }),
        (
            "Type" | "ClassType" | "ShapeType" | "EnumType" | "InterfaceType" | "SingleType"
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
        ("ClassType" | "ShapeType" | "SingleType" | "AnnotationType", "fields") => {
            Some(ir::Type::Function {
                params: Vec::new(),
                ret: Box::new(ir::Type::list(ir::Type::named("Field"))),
            })
        }
        ("ClassType" | "SingleType", "field") => Some(ir::Type::Function {
            params: vec![ir::Type::Str],
            ret: Box::new(ir::Type::option(ir::Type::named("Field"))),
        }),
        ("ClassType" | "ShapeType" | "EnumType" | "InterfaceType" | "SingleType", "methods") => {
            Some(ir::Type::Function {
                params: Vec::new(),
                ret: Box::new(ir::Type::list(ir::Type::named("Method"))),
            })
        }
        ("ClassType" | "SingleType", "method") => Some(ir::Type::Function {
            params: vec![ir::Type::Str],
            ret: Box::new(ir::Type::option(ir::Type::named("Method"))),
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
        ("Method", "invoke") => Some(ir::Type::Function {
            params: vec![
                ir::Type::named("Any"),
                ir::Type::list(ir::Type::named("Any")),
            ],
            ret: Box::new(ir::Type::named("Any")),
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
        ("List" | "Array", "head" | "first" | "last" | "removeFirst" | "removeLast") => {
            Some(ir::Type::Function {
                params: Vec::new(),
                ret: Box::new(ir::Type::option(item)),
            })
        }
        ("List" | "Array", "get" | "remove") => Some(ir::Type::Function {
            params: vec![ir::Type::Int],
            ret: Box::new(ir::Type::option(item)),
        }),
        ("List" | "Array" | "Set" | "Map" | "Str", "size" | "length") => Some(ir::Type::Function {
            params: Vec::new(),
            ret: Box::new(ir::Type::Int),
        }),
        ("List" | "Array" | "Set" | "Map", "isEmpty" | "nonEmpty") => Some(ir::Type::Function {
            params: Vec::new(),
            ret: Box::new(ir::Type::Bool),
        }),
        _ => None,
    }
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
                    | "SingleType"
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
        ir::Type::Named { args, .. } | ir::Type::Tuple(args) => {
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

fn infer_ir_type_subst(
    expected: &ir::Type,
    actual: &ir::Type,
    subst: &mut HashMap<String, ir::Type>,
) {
    match expected {
        ir::Type::TypeParam(name) => {
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

fn ir_type_assignable(program: &ir::Program, actual: &ir::Type, expected: &ir::Type) -> bool {
    let mut seen = Vec::new();
    ir_type_assignable_inner(program, actual, expected, &mut seen)
}

fn ir_type_assignable_inner(
    program: &ir::Program,
    actual: &ir::Type,
    expected: &ir::Type,
    seen: &mut Vec<(String, String)>,
) -> bool {
    if actual == expected
        || matches!(actual, ir::Type::Never | ir::Type::Unknown)
        || matches!(expected, ir::Type::Unknown)
    {
        return true;
    }
    let (
        ir::Type::Named {
            name: actual_name,
            args: actual_args,
        },
        ir::Type::Named {
            name: expected_name,
            ..
        },
    ) = (actual, expected)
    else {
        return false;
    };
    let key = (actual_name.clone(), expected_name.clone());
    if seen.iter().any(|item| item == &key) {
        return false;
    }
    seen.push(key);
    let Some(actual_def) = program.types.iter().find(|ty| ty.name == *actual_name) else {
        return false;
    };
    let subst = ir_type_subst(actual_def, actual_args);
    actual_def.with_bounds.iter().any(|bound| {
        let bound_ty = substitute_ir_type(bound, &subst);
        ir_type_assignable_inner(program, &bound_ty, expected, seen)
    })
}

fn describe_ir_type(ty: &ir::Type) -> String {
    match ty {
        ir::Type::Unknown => "<unknown>".to_string(),
        ir::Type::Never => "Never".to_string(),
        ir::Type::Unit => "Unit".to_string(),
        ir::Type::Bool => "Bool".to_string(),
        ir::Type::Int => "Int".to_string(),
        ir::Type::Float => "Float".to_string(),
        ir::Type::Str => "Str".to_string(),
        ir::Type::Named { name, args } if args.is_empty() => name.clone(),
        ir::Type::Named { name, args } => format!(
            "{}[{}]",
            name,
            args.iter()
                .map(describe_ir_type)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        ir::Type::Tuple(items) => format!(
            "({})",
            items
                .iter()
                .map(describe_ir_type)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        ir::Type::Record(fields) => format!(
            "{{{}}}",
            fields
                .iter()
                .map(|field| format!("{} {}", field.name, describe_ir_type(&field.ty)))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        ir::Type::Function { params, ret } => format!(
            "({}) -> {}",
            params
                .iter()
                .map(describe_ir_type)
                .collect::<Vec<_>>()
                .join(", "),
            describe_ir_type(ret)
        ),
        ir::Type::TypeParam(name) => name.clone(),
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

fn is_named_runtime_value_path(program: &ir::Program, path: &[String]) -> bool {
    if path.is_empty() {
        return false;
    }

    if path.len() >= 2 && explicit_enum_case_value_exists(program, &path[0], &path[1]) {
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
    if matches!(path, [owner, method] if ((owner == "Int" || owner == "Float") && method == "parse")
        || (owner == "Option" && method == "when"))
    {
        return true;
    }
    if path.len() == 1 {
        return builtin_callable_root_name(&path[0])
            || declared_type_exists(program, &path[0])
            || unique_bare_enum_case_exists(program, &path[0]);
    }
    explicit_enum_case_exists(program, &path[0], &path[1])
        || single_type_exists(program, &path[0])
        || builtin_callable_root_name(&path[0])
        || declared_type_exists(program, &path[0])
}

fn declared_type_exists(program: &ir::Program, name: &str) -> bool {
    program.types.iter().any(|ty| ty.name == name)
}

fn builtin_callable_root_name(name: &str) -> bool {
    matches!(
        name,
        "OS" | "Range"
            | "List"
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

fn single_type_exists(program: &ir::Program, name: &str) -> bool {
    program
        .types
        .iter()
        .any(|ty| ty.kind == ast::TypeKind::Single && ty.name == name)
}

fn builtin_zero_arg_value_name(name: &str) -> bool {
    matches!(name, "None")
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

fn enum_case_pattern_fields(case: &ir::EnumCase, arity: usize) -> Option<Vec<String>> {
    if arity > case.fields.len() {
        return None;
    }
    if case.fields[arity..]
        .iter()
        .any(|field| field.initializer.is_none())
    {
        return None;
    }
    Some(
        case.fields[..arity]
            .iter()
            .map(|field| field.name.clone())
            .collect(),
    )
}

fn lower_if_stmt_else_expr(branch: &ElseBranch) -> Option<ElseExprBranch> {
    match branch {
        ElseBranch::Block(block) => Some(ElseExprBranch::Block(block.clone())),
        ElseBranch::If(stmt) => {
            let condition = stmt.condition.clone()?;
            if !stmt.condition_clauses.is_empty()
                || !stmt.pattern_clauses.is_empty()
                || stmt.pattern.is_some()
                || stmt.pattern_value.is_some()
                || !stmt.bindings.is_empty()
                || stmt.binding_value.is_some()
            {
                return None;
            }
            let else_branch = lower_if_stmt_else_expr(stmt.else_branch.as_ref()?)?;
            Some(ElseExprBranch::If(Box::new(Expr::If {
                condition: Box::new(condition),
                then_block: stmt.then_block.clone(),
                else_branch: Box::new(else_branch),
                span: stmt.span,
            })))
        }
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
    fn lowers_types_and_methods() {
        let program = parse_inline(
            r#"
            class Counter {
                value Int
            }

            impl Counter {
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
            }

            impl Counter {
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

            class Runner {}

            impl Runner {
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
            class Amount {
                amount Int
                description Str
            }

            def main() Int {
                base = 10
                inc (Int) -> Int = value -> value + 1
                plus = (value Int) -> value + base

                def add(value Int) Int = plus(value)

                current = Amount(1, "a")
                updated = current :< { amount: add(inc(1)) }
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
    fn lowers_map_constructor_pairs_without_binary_operator_errors() {
        let program = parse_inline(
            r#"
            def main() Unit {
                entries = Map("a": 1, "bbb": 2)
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
}
