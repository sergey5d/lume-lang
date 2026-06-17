use std::collections::HashMap;

use crate::{
    ast::{
        self, BinaryOp as AstBinaryOp, ImplBlock, ImplTargetKind, Item, TypeDecl, TypeMember,
    },
    core::{
        self, AssignOp, Block, CallableBody, DestructureKind, ElseBranch, ElseExprBranch, Expr,
        FunctionDecl, MatchCase, MatchCaseBody, MethodDecl, Pattern, Stmt, TypeRef,
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
            global_inits: Vec::new(),
        }
    }

    fn lower(&mut self) -> ir::Program {
        self.declare_top_level_items();
        self.define_items();
        self.lower_top_level_functions();
        self.lower_methods();
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
        for item in &self.source.items {
            let Item::Impl(block) = item else {
                continue;
            };
            if block.target_kind == ImplTargetKind::Single {
                self.declare_synthetic_single(block);
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

        for member in &decl.members {
            match member {
                TypeMember::Field(field) => {
                    let ty = field
                        .ty
                        .as_ref()
                        .map(lower_type_ref)
                        .unwrap_or(ir::Type::Unknown);
                    fields.push(ir::Field {
                        visibility: field.visibility,
                        mutable: field.mutable,
                        name: field.name.clone(),
                        ty: ty.clone(),
                        initializer: lower_field_initializer_constant(field.initializer.as_ref()),
                        span: Some(field.span),
                    });
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
                            visibility: field.visibility,
                            mutable: field.mutable,
                            name: field.name.clone(),
                            ty: field
                                .ty
                                .as_ref()
                                .map(lower_type_ref)
                                .unwrap_or(ir::Type::Unknown),
                            initializer: lower_field_initializer_constant(
                                field.initializer.as_ref(),
                            ),
                            span: Some(field.span),
                        })
                        .collect();
                    cases.push(ir::EnumCase {
                        name: case.name.clone(),
                        fields: case_fields,
                        span: Some(case.span),
                    });
                }
            }
        }

        if let Some(ty) = self.program.types.get_mut(type_id.0) {
            ty.fields = fields;
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
        let Some(type_id) = self.impl_target_type_id(target_name, block.target_kind)
        else {
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

    fn declare_synthetic_single(&mut self, block: &ImplBlock) {
        let Some(target_name) = named_type_name(&block.target) else {
            return;
        };
        let key = (target_name.to_string(), ast::TypeKind::Object);
        if self.type_ids.contains_key(&key) {
            return;
        }

        // `impl single Name` can attach singleton methods to an explicit
        // `single Name { ... }` declaration or synthesize an empty companion.
        let mut ty = ir::TypeDef::new(ast::TypeKind::Object, target_name.to_string());
        if let Some(base_decl) = self.source.items.iter().find_map(|item| match item {
            Item::Type(decl) if decl.name == target_name && decl.kind != ast::TypeKind::Object => {
                Some(decl)
            }
            _ => None,
        }) {
            ty.visibility = base_decl.visibility;
            ty.span = Some(base_decl.span);
        } else {
            ty.span = Some(block.span);
        }
        let id = self.program.add_type(ty);
        self.type_ids.insert(key, id);
    }

    fn impl_target_type_id(
        &self,
        target_name: &str,
        target_kind: ImplTargetKind,
    ) -> Option<ir::TypeId> {
        match target_kind {
            ImplTargetKind::Single => self
                .type_ids
                .get(&(target_name.to_string(), ast::TypeKind::Object))
                .copied(),
            ImplTargetKind::Instance => self.type_ids.iter().find_map(|((name, kind), id)| {
                (name == target_name && *kind != ast::TypeKind::Object).then_some(*id)
            }),
        }
    }

    fn declare_function(
        &mut self,
        name: &str,
        visibility: ast::Visibility,
        type_params: &[ast::TypeParam],
        return_type: Option<&TypeRef>,
        kind: ir::FunctionKind,
        params: &[core::Param],
        this_local: Option<(String, ir::Type)>,
        span: Span,
    ) -> ir::FunctionId {
        let mut function = ir::Function::new(
            name.to_string(),
            kind,
            return_type.map(lower_type_ref).unwrap_or(ir::Type::Unknown),
        );
        function.visibility = visibility;
        function.type_params = type_params.iter().map(|param| param.name.clone()).collect();
        function.span = Some(span);
        if let Some((this_name, this_ty)) = this_local {
            function.add_local(this_name, this_ty, false, ir::LocalKind::Capture);
        }
        for param in params {
            function.add_param(
                param.name.clone(),
                param
                    .ty
                    .as_ref()
                    .map(lower_type_ref)
                    .unwrap_or(ir::Type::Unknown),
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

    fn with_capture_sources(
        mut self,
        capture_sources: HashMap<String, CaptureSource>,
    ) -> Self {
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
                    let ty = local
                        .ty
                        .as_ref()
                        .map(lower_type_ref)
                        .unwrap_or(ir::Type::Unknown);
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
                    let value = if assignment.operator == AssignOp::Reassign {
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
        for param in &function.params {
            nested.add_param(
                param.name.clone(),
                param
                    .ty
                    .as_ref()
                    .map(lower_type_ref)
                    .unwrap_or(ir::Type::Unknown),
            );
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
        for param in params {
            nested.add_param(
                param.name.clone(),
                param
                    .ty
                    .as_ref()
                    .map(lower_type_ref)
                    .unwrap_or(ir::Type::Unknown),
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
                    if param.name != "_" {
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

    fn lower_placeholder_lambda(&mut self, expr: &Expr) -> ir::Operand {
        let param_name = format!("v{}", self.function().locals.len() + 1);
        let rewritten = rewrite_placeholder_expr(expr, &param_name);
        let rvalue = self.lower_lambda_rvalue(
            &[core::LambdaParam {
                name: param_name,
                ty: None,
                span: expr.span(),
            }],
            &rewritten,
            expr.span(),
        );
        let temp = self.add_temp(ir::Type::Unknown);
        self.push_statement(ir::Statement {
            span: Some(expr.span()),
            kind: ir::StatementKind::Assign {
                target: ir::Place::Local(temp),
                value: rvalue,
            },
        });
        ir::Operand::Copy(Box::new(ir::Place::Local(temp)))
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
        for param in params {
            nested.add_param(
                param.name.clone(),
                param
                    .ty
                    .as_ref()
                    .map(lower_type_ref)
                    .unwrap_or(ir::Type::Unknown),
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

    fn lower_anonymous_interface_rvalue(&mut self, methods: &[MethodDecl]) -> ir::RValue {
        let fields = methods
            .iter()
            .map(|method| {
                let ty = ir::Type::Function {
                    params: method
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
                        method
                            .return_type
                            .as_ref()
                            .map(lower_type_ref)
                            .unwrap_or(ir::Type::Unknown),
                    ),
                };
                let closure = self.lower_callable_closure(
                    &method.name,
                    &method.params,
                    method.return_type.as_ref(),
                    method.body.as_ref(),
                    method.span,
                );
                let operand = self.emit_temp_from_rvalue(closure, ty, Some(method.span));
                ir::NamedOperand {
                    name: method.name.clone(),
                    value: operand,
                }
            })
            .collect();
        ir::RValue::Record(fields)
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
        if !stmt.clauses.is_empty() {
            let failure_block = self.add_block();
            let continue_block = self.add_block();

            self.lower_refutable_clause_chain(&stmt.clauses, continue_block, failure_block);

            self.current_block = Some(failure_block);
            self.emit_panic("let pattern did not match", stmt.span);
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
        self.emit_panic("let pattern did not match", stmt.span);
        self.terminate(ir::Terminator {
            span: Some(stmt.span),
            kind: ir::TerminatorKind::Unreachable,
        });

        self.current_block = Some(continue_block);
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
                    method: "unwrap".to_string(),
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

                self.current_block = Some(failure_block);
                self.emit_panic("for pattern did not match", first.span);
                self.terminate(ir::Terminator {
                    span: Some(first.span),
                    kind: ir::TerminatorKind::Unreachable,
                });
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

    fn destructure_field_names(&self, expr: &Expr, bindings: &[ast::Binding]) -> Vec<String> {
        let positional_fields = if let Expr::Call { callee, .. } = expr {
            if let Some(path) = expr_path(callee) {
                self.lookup_destructured_type_fields(&path, bindings.len())
                    .map(|(_, fields)| fields)
            } else {
                None
            }
        } else {
            None
        };
        bindings
            .iter()
            .enumerate()
            .map(|(index, binding)| {
                binding
                    .field_name
                    .clone()
                    .or_else(|| {
                        positional_fields
                            .as_ref()
                            .and_then(|fields| fields.get(index).cloned())
                    })
                    .unwrap_or_else(|| format!("_{}", index + 1))
            })
            .collect()
    }

    fn destructure_field_names_from_bindings(&self, bindings: &[ast::Binding]) -> Vec<String> {
        bindings
            .iter()
            .enumerate()
            .map(|(index, binding)| {
                binding
                    .field_name
                    .clone()
                    .unwrap_or_else(|| format!("_{}", index + 1))
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
                ir::RValue::Call {
                    callee: ir::Callee::Named {
                        path: vec!["None".to_string()],
                    },
                    args: Vec::new(),
                    structural: false,
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
        if arity == 0 {
            if path.len() >= 2 {
                let type_name = &path[path.len() - 2];
                let has_case = self
                    .program
                    .types
                    .iter()
                    .filter(|ty| ty.kind == ast::TypeKind::Enum && ty.name == *type_name)
                    .flat_map(|ty| ty.enum_cases.iter())
                    .any(|case| case.name == *case_name);
                if has_case {
                    return Some(Vec::new());
                }
            }
            if matches!(
                case_name.as_str(),
                "None" | "Some" | "Ok" | "Err" | "Left" | "Right"
            ) {
                return Some(Vec::new());
            }
        }
        match case_name.as_str() {
            "Some" | "Ok" | "Left" | "Right" => {
                return Some(vec!["value".to_string()]);
            }
            "Err" => {
                return Some(vec!["error".to_string()]);
            }
            "None" => return Some(Vec::new()),
            _ => {}
        }

        if path.len() >= 2 {
            let key = format!("{}.{}", path[path.len() - 2], case_name);
            if let Some(fields) = self.case_fields.get(&key) {
                return Some(fields.clone());
            }
        }

        let matches = self
            .case_fields
            .iter()
            .filter_map(|(key, fields)| {
                if key.ends_with(&format!(".{case_name}")) {
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
        if should_lower_as_placeholder_lambda(expr) {
            return self.lower_placeholder_lambda(expr);
        }
        match expr {
            Expr::Identifier { name, span } => self.lookup_value(name).unwrap_or_else(|| {
                let path = vec![name.clone()];
                if is_named_runtime_value_path(self.program, &path) {
                    return self.emit_temp_from_rvalue(
                        ir::RValue::Call {
                            callee: ir::Callee::Named { path },
                            args: Vec::new(),
                            structural: false,
                        },
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
            Expr::ListLiteral { .. }
            | Expr::TupleLiteral { .. }
            | Expr::RecordLiteral { .. }
            | Expr::AnonymousInterface { .. }
            | Expr::Unary { .. }
            | Expr::Binary { .. }
            | Expr::Call { .. }
            | Expr::Member { .. }
            | Expr::Index { .. }
            | Expr::RecordUpdate { .. }
            | Expr::Is { .. }
            | Expr::Lambda { .. } => self.lower_expr_from_rvalue(expr),
            Expr::Placeholder { span } => {
                self.add_error(
                    "lower_invariant",
                    "bare placeholder expression should be rewritten before lowering",
                    *span,
                );
                ir::Operand::Const(ir::Constant::Unit)
            }
        }
    }

    fn lower_expr_from_rvalue(&mut self, expr: &Expr) -> ir::Operand {
        let rvalue = self
            .lower_rvalue(expr)
            .expect("rvalue-backed expression should lower to an rvalue");
        let temp = self.add_temp(ir::Type::Unknown);
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
                    method: "unwrap".to_string(),
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
                return Some(ir::RValue::Call {
                    callee: ir::Callee::Named { path },
                    args: Vec::new(),
                    structural: false,
                });
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
            } => Some(ir::RValue::Call {
                callee: self.lower_callee(callee),
                args: self
                    .reorder_call_args(callee, args)
                    .into_iter()
                    .map(|arg| self.lower_expr(&arg.value))
                    .collect(),
                structural: call_uses_structural_record_arg(args, *style),
            }),
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
            Expr::Lambda { params, body, span } => {
                Some(self.lower_lambda_rvalue(params, body, *span))
            }
            Expr::AnonymousInterface { methods, .. } => {
                Some(self.lower_anonymous_interface_rvalue(methods))
            }
            Expr::RecordUpdate {
                receiver, updates, ..
            } => Some(ir::RValue::RecordUpdate {
                base: self.lower_expr(receiver),
                updates: updates
                    .iter()
                    .map(|update| ir::NamedOperand {
                        name: update.name.clone().unwrap_or_default(),
                        value: self.lower_expr(&update.value),
                    })
                    .collect(),
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
            AstBinaryOp::Append => self.lower_operator_method_rvalue(left, ":+", right),
            AstBinaryOp::Concat => self.lower_operator_method_rvalue(left, "++", right),
            AstBinaryOp::Remove => self.lower_operator_method_rvalue(left, "--", right),
            AstBinaryOp::Prepend => self.lower_operator_method_rvalue(left, ":-", right),
            AstBinaryOp::Compose => self.lower_operator_method_rvalue(left, "::", right),
            _ => ir::RValue::Binary {
                op: map_binary_op(op).expect("non-special binary operator should map to IR"),
                left: self.lower_expr(left),
                right: self.lower_expr(right),
            },
        }
    }

    fn lower_operator_method_rvalue(
        &mut self,
        left: &Expr,
        method: &str,
        right: &Expr,
    ) -> ir::RValue {
        ir::RValue::Call {
            callee: ir::Callee::Method {
                receiver: self.lower_expr(left),
                method: method.to_string(),
            },
            args: vec![self.lower_expr(right)],
            structural: false,
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
                    ("List", "from") | ("Set", "from")
                ) {
                    return Some(vec!["values".to_string()]);
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
        let _ = has_explicit_constructor;
        None
    }

    fn lower_place(&mut self, expr: &Expr) -> Option<ir::Place> {
        match expr {
            Expr::Identifier { name, span } => self.lookup_place(name).or_else(|| {
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

    fn lookup_place(&mut self, name: &str) -> Option<ir::Place> {
        for scope in self.scopes.iter().rev() {
            if let Some(local) = scope.get(name).copied() {
                return Some(ir::Place::Local(local));
            }
        }
        if let Some(global) = self.globals.get(name).copied() {
            return Some(ir::Place::Global(global));
        }
        if let Some(local) = self.capture_local(name) {
            return Some(ir::Place::Local(local));
        }
        None
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
        _ => None,
    }
}

fn map_assign_op(op: AssignOp) -> Option<ir::BinaryOp> {
    match op {
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
        AstBinaryOp::BitOr => Some(ir::BinaryOp::BitOr),
        AstBinaryOp::BitAnd => Some(ir::BinaryOp::BitAnd),
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
        AstBinaryOp::Concat => Some(ir::BinaryOp::Concat),
        AstBinaryOp::Colon
        | AstBinaryOp::Remove
        | AstBinaryOp::Append
        | AstBinaryOp::Prepend
        | AstBinaryOp::Compose => None,
    }
}

fn lower_type_ref(reference: &TypeRef) -> ir::Type {
    match reference {
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

    if object_type_exists(program, &path[0]) {
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
    if path.len() == 1 {
        return builtin_callable_root_name(&path[0])
            || declared_type_exists(program, &path[0])
            || unique_bare_enum_case_exists(program, &path[0]);
    }
    explicit_enum_case_exists(program, &path[0], &path[1])
        || object_type_exists(program, &path[0])
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

fn object_type_exists(program: &ir::Program, name: &str) -> bool {
    program
        .types
        .iter()
        .any(|ty| ty.kind == ast::TypeKind::Object && ty.name == name)
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

fn contains_placeholder_expr(expr: &Expr) -> bool {
    match expr {
        Expr::Placeholder { .. } => true,
        Expr::ListLiteral { items, .. } | Expr::TupleLiteral { items, .. } => {
            items.iter().any(contains_placeholder_expr)
        }
        Expr::Call { callee, args, .. } => {
            contains_placeholder_expr(callee)
                || args.iter().any(|arg| contains_placeholder_expr(&arg.value))
        }
        Expr::Member { receiver, .. } => contains_placeholder_expr(receiver),
        Expr::Index {
            receiver, index, ..
        } => contains_placeholder_expr(receiver) || contains_placeholder_expr(index),
        Expr::RecordUpdate {
            receiver, updates, ..
        } => {
            contains_placeholder_expr(receiver)
                || updates
                    .iter()
                    .any(|update| contains_placeholder_expr(&update.value))
        }
        Expr::RecordLiteral { fields, .. } => fields
            .iter()
            .any(|field| contains_placeholder_expr(&field.value)),
        Expr::AnonymousInterface { methods, .. } => methods
            .iter()
            .any(|method| body_contains_placeholder(&method.body)),
        Expr::Unary { expr, .. } => contains_placeholder_expr(expr),
        Expr::Binary { left, right, .. } => {
            contains_placeholder_expr(left) || contains_placeholder_expr(right)
        }
        Expr::Is { left, .. } => contains_placeholder_expr(left),
        Expr::If {
            condition,
            then_block,
            else_branch,
            ..
        } => {
            contains_placeholder_expr(condition)
                || block_contains_placeholder(then_block)
                || match else_branch.as_ref() {
                    ElseExprBranch::If(expr) => contains_placeholder_expr(expr),
                    ElseExprBranch::Block(block) => block_contains_placeholder(block),
                }
        }
        Expr::Block { body, .. } => block_contains_placeholder(body),
        Expr::Match { value, cases, .. } => {
            contains_placeholder_expr(value)
                || cases.iter().any(|case| match &case.body {
                    MatchCaseBody::Expr(expr) => contains_placeholder_expr(expr),
                    MatchCaseBody::Block(block) => block_contains_placeholder(block),
                })
        }
        Expr::ForYield {
            bindings,
            yield_body,
            ..
        } => {
            bindings.iter().any(|binding| {
                binding
                    .iterable
                    .as_ref()
                    .is_some_and(contains_placeholder_expr)
                    || binding.values.iter().any(contains_placeholder_expr)
            }) || block_contains_placeholder(yield_body)
        }
        Expr::Try { value, .. } => contains_placeholder_expr(value),
        Expr::Lambda { .. } => false,
        Expr::Identifier { .. }
        | Expr::Integer { .. }
        | Expr::Float { .. }
        | Expr::String { .. }
        | Expr::Bool { .. }
        | Expr::Unit { .. } => false,
    }
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

fn param_names_from_function(function: &ir::Function) -> Vec<String> {
    function
        .params
        .iter()
        .filter_map(|param| function.locals.get(param.0))
        .map(|local| local.name.clone())
        .collect()
}

fn should_lower_as_placeholder_lambda(expr: &Expr) -> bool {
    if matches!(expr, Expr::Lambda { .. }) || !contains_placeholder_expr(expr) {
        return false;
    }
    match expr {
        Expr::Call { callee, .. } => contains_placeholder_expr(callee),
        Expr::RecordUpdate { receiver, .. } => contains_placeholder_expr(receiver),
        Expr::Member { receiver, .. } => contains_placeholder_expr(receiver),
        Expr::Index { receiver, .. } => contains_placeholder_expr(receiver),
        _ => true,
    }
}

fn rewrite_placeholder_expr(expr: &Expr, name: &str) -> Expr {
    match expr {
        Expr::Placeholder { span } => Expr::Identifier {
            name: name.to_string(),
            span: *span,
        },
        Expr::ListLiteral { items, span } => Expr::ListLiteral {
            items: items
                .iter()
                .map(|item| rewrite_placeholder_expr(item, name))
                .collect(),
            span: *span,
        },
        Expr::TupleLiteral { items, span } => Expr::TupleLiteral {
            items: items
                .iter()
                .map(|item| rewrite_placeholder_expr(item, name))
                .collect(),
            span: *span,
        },
        Expr::Call {
            callee,
            args,
            style,
            span,
        } => Expr::Call {
            callee: Box::new(rewrite_placeholder_expr(callee, name)),
            args: args
                .iter()
                .map(|arg| core::CallArg {
                    name: arg.name.clone(),
                    value: rewrite_placeholder_expr(&arg.value, name),
                    span: arg.span,
                })
                .collect(),
            style: *style,
            span: *span,
        },
        Expr::Member {
            receiver,
            name: member,
            span,
        } => Expr::Member {
            receiver: Box::new(rewrite_placeholder_expr(receiver, name)),
            name: member.clone(),
            span: *span,
        },
        Expr::Index {
            receiver,
            index,
            span,
        } => Expr::Index {
            receiver: Box::new(rewrite_placeholder_expr(receiver, name)),
            index: Box::new(rewrite_placeholder_expr(index, name)),
            span: *span,
        },
        Expr::RecordUpdate {
            receiver,
            updates,
            span,
        } => Expr::RecordUpdate {
            receiver: Box::new(rewrite_placeholder_expr(receiver, name)),
            updates: updates
                .iter()
                .map(|update| core::CallArg {
                    name: update.name.clone(),
                    value: rewrite_placeholder_expr(&update.value, name),
                    span: update.span,
                })
                .collect(),
            span: *span,
        },
        Expr::RecordLiteral {
            fields,
            values,
            span,
        } => Expr::RecordLiteral {
            fields: fields
                .iter()
                .map(|field| core::CallArg {
                    name: field.name.clone(),
                    value: rewrite_placeholder_expr(&field.value, name),
                    span: field.span,
                })
                .collect(),
            values: values
                .iter()
                .map(|value| rewrite_placeholder_expr(value, name))
                .collect(),
            span: *span,
        },
        Expr::AnonymousInterface {
            interfaces,
            methods,
            span,
        } => Expr::AnonymousInterface {
            interfaces: interfaces.clone(),
            methods: methods
                .iter()
                .map(|method| core::MethodDecl {
                    annotations: method.annotations.clone(),
                    visibility: method.visibility,
                    name: method.name.clone(),
                    type_params: method.type_params.clone(),
                    params: method.params.clone(),
                    return_type: method.return_type.clone(),
                    body: method
                        .body
                        .as_ref()
                        .map(|body| rewrite_callable_body(body, name)),
                    span: method.span,
                })
                .collect(),
            span: *span,
        },
        Expr::Unary { op, expr, span } => Expr::Unary {
            op: *op,
            expr: Box::new(rewrite_placeholder_expr(expr, name)),
            span: *span,
        },
        Expr::Binary {
            left,
            op,
            right,
            span,
        } => Expr::Binary {
            left: Box::new(rewrite_placeholder_expr(left, name)),
            op: *op,
            right: Box::new(rewrite_placeholder_expr(right, name)),
            span: *span,
        },
        Expr::Is { left, target, span } => Expr::Is {
            left: Box::new(rewrite_placeholder_expr(left, name)),
            target: target.clone(),
            span: *span,
        },
        Expr::If {
            condition,
            then_block,
            else_branch,
            span,
        } => Expr::If {
            condition: Box::new(rewrite_placeholder_expr(condition, name)),
            then_block: rewrite_block(then_block, name),
            else_branch: Box::new(match else_branch.as_ref() {
                ElseExprBranch::If(expr) => {
                    ElseExprBranch::If(Box::new(rewrite_placeholder_expr(expr, name)))
                }
                ElseExprBranch::Block(block) => ElseExprBranch::Block(rewrite_block(block, name)),
            }),
            span: *span,
        },
        Expr::Block { body, span } => Expr::Block {
            body: rewrite_block(body, name),
            span: *span,
        },
        Expr::Match {
            partial,
            value,
            cases,
            span,
        } => Expr::Match {
            partial: *partial,
            value: Box::new(rewrite_placeholder_expr(value, name)),
            cases: rewrite_match_cases(cases, name),
            span: *span,
        },
        Expr::ForYield {
            bindings,
            yield_body,
            span,
        } => Expr::ForYield {
            bindings: bindings
                .iter()
                .map(|binding| core::ForBinding {
                    bindings: binding.bindings.clone(),
                    destructure: binding.destructure,
                    pattern: binding.pattern.clone(),
                    iterable: binding
                        .iterable
                        .as_ref()
                        .map(|iterable| rewrite_placeholder_expr(iterable, name)),
                    values: binding
                        .values
                        .iter()
                        .map(|value| rewrite_placeholder_expr(value, name))
                        .collect(),
                    span: binding.span,
                })
                .collect(),
            yield_body: rewrite_block(yield_body, name),
            span: *span,
        },
        Expr::Try { value, span } => Expr::Try {
            value: Box::new(rewrite_placeholder_expr(value, name)),
            span: *span,
        },
        Expr::Lambda { .. } => expr.clone(),
        Expr::Identifier { .. }
        | Expr::Integer { .. }
        | Expr::Float { .. }
        | Expr::String { .. }
        | Expr::Bool { .. }
        | Expr::Unit { .. } => expr.clone(),
    }
}

fn rewrite_callable_body(body: &CallableBody, name: &str) -> CallableBody {
    match body {
        CallableBody::Expr(expr) => CallableBody::Expr(rewrite_placeholder_expr(expr, name)),
        CallableBody::Block(block) => CallableBody::Block(rewrite_block(block, name)),
    }
}

fn rewrite_block(block: &Block, name: &str) -> Block {
    Block {
        statements: block
            .statements
            .iter()
            .map(|stmt| rewrite_stmt(stmt, name))
            .collect(),
        span: block.span,
    }
}

fn rewrite_stmt(stmt: &Stmt, name: &str) -> Stmt {
    match stmt {
        Stmt::Binding(binding) => Stmt::Binding(core::BindingStmt {
            visibility: binding.visibility,
            bindings: binding.bindings.clone(),
            values: binding
                .values
                .iter()
                .map(|value| rewrite_placeholder_expr(value, name))
                .collect(),
            destructure: binding.destructure,
            span: binding.span,
        }),
        Stmt::PatternBinding(stmt) => Stmt::PatternBinding(core::PatternBindingStmt {
            clauses: stmt
                .clauses
                .iter()
                .map(|clause| core::RefutableClause {
                    pattern: clause.pattern.clone(),
                    value: rewrite_placeholder_expr(&clause.value, name),
                    span: clause.span,
                })
                .collect(),
            pattern: stmt.pattern.clone(),
            value: rewrite_placeholder_expr(&stmt.value, name),
            span: stmt.span,
        }),
        Stmt::Assignment(assignment) => Stmt::Assignment(core::AssignmentStmt {
            targets: assignment
                .targets
                .iter()
                .map(|target| rewrite_placeholder_expr(target, name))
                .collect(),
            operator: assignment.operator,
            values: assignment
                .values
                .iter()
                .map(|value| rewrite_placeholder_expr(value, name))
                .collect(),
            span: assignment.span,
        }),
        Stmt::If(stmt) => Stmt::If(core::IfStmt {
            condition: stmt
                .condition
                .as_ref()
                .map(|condition| rewrite_placeholder_expr(condition, name)),
            condition_clauses: stmt
                .condition_clauses
                .iter()
                .map(|clause| match clause {
                    core::IfConditionClause::Let(clause) => {
                        core::IfConditionClause::Let(core::RefutableClause {
                            pattern: clause.pattern.clone(),
                            value: rewrite_placeholder_expr(&clause.value, name),
                            span: clause.span,
                        })
                    }
                    core::IfConditionClause::Expr(condition) => {
                        core::IfConditionClause::Expr(rewrite_placeholder_expr(condition, name))
                    }
                })
                .collect(),
            pattern: stmt.pattern.clone(),
            pattern_value: stmt
                .pattern_value
                .as_ref()
                .map(|value| rewrite_placeholder_expr(value, name)),
            pattern_clauses: stmt
                .pattern_clauses
                .iter()
                .map(|clause| core::RefutableClause {
                    pattern: clause.pattern.clone(),
                    value: rewrite_placeholder_expr(&clause.value, name),
                    span: clause.span,
                })
                .collect(),
            bindings: stmt.bindings.clone(),
            binding_value: stmt
                .binding_value
                .as_ref()
                .map(|value| rewrite_placeholder_expr(value, name)),
            then_block: rewrite_block(&stmt.then_block, name),
            else_branch: stmt.else_branch.as_ref().map(|branch| match branch {
                ElseBranch::If(stmt) => ElseBranch::If(Box::new(
                    match rewrite_stmt(&Stmt::If((**stmt).clone()), name) {
                        Stmt::If(stmt) => stmt,
                        _ => unreachable!(),
                    },
                )),
                ElseBranch::Block(block) => ElseBranch::Block(rewrite_block(block, name)),
            }),
            span: stmt.span,
        }),
        Stmt::LetElse(stmt) => Stmt::LetElse(core::LetElseStmt {
            clauses: stmt
                .clauses
                .iter()
                .map(|clause| core::RefutableClause {
                    pattern: clause.pattern.clone(),
                    value: rewrite_placeholder_expr(&clause.value, name),
                    span: clause.span,
                })
                .collect(),
            pattern: stmt.pattern.clone(),
            value: rewrite_placeholder_expr(&stmt.value, name),
            else_block: rewrite_block(&stmt.else_block, name),
            span: stmt.span,
        }),
        Stmt::Match(stmt) => Stmt::Match(core::MatchStmt {
            partial: stmt.partial,
            value: rewrite_placeholder_expr(&stmt.value, name),
            cases: rewrite_match_cases(&stmt.cases, name),
            span: stmt.span,
        }),
        Stmt::While(stmt) => Stmt::While(core::WhileStmt {
            condition: rewrite_placeholder_expr(&stmt.condition, name),
            body: rewrite_block(&stmt.body, name),
            span: stmt.span,
        }),
        Stmt::For(stmt) => Stmt::For(core::ForStmt {
            bindings: stmt
                .bindings
                .iter()
                .map(|binding| core::ForBinding {
                    bindings: binding.bindings.clone(),
                    destructure: binding.destructure,
                    pattern: binding.pattern.clone(),
                    iterable: binding
                        .iterable
                        .as_ref()
                        .map(|iterable| rewrite_placeholder_expr(iterable, name)),
                    values: binding
                        .values
                        .iter()
                        .map(|value| rewrite_placeholder_expr(value, name))
                        .collect(),
                    span: binding.span,
                })
                .collect(),
            body: rewrite_block(&stmt.body, name),
            span: stmt.span,
        }),
        Stmt::Return(stmt) => Stmt::Return(core::ReturnStmt {
            value: stmt
                .value
                .as_ref()
                .map(|value| rewrite_placeholder_expr(value, name)),
            span: stmt.span,
        }),
        Stmt::Break(stmt) => Stmt::Break(stmt.clone()),
        Stmt::Continue(stmt) => Stmt::Continue(stmt.clone()),
        Stmt::Expr(stmt) => Stmt::Expr(core::ExprStmt {
            expr: rewrite_placeholder_expr(&stmt.expr, name),
            span: stmt.span,
        }),
        Stmt::LocalFunction(function) => Stmt::LocalFunction(core::FunctionDecl {
            annotations: function.annotations.clone(),
            visibility: function.visibility,
            name: function.name.clone(),
            type_params: function.type_params.clone(),
            params: function.params.clone(),
            return_type: function.return_type.clone(),
            body: rewrite_callable_body(&function.body, name),
            span: function.span,
        }),
    }
}

fn rewrite_match_cases(cases: &[MatchCase], name: &str) -> Vec<MatchCase> {
    cases
        .iter()
        .map(|case| MatchCase {
            pattern: case.pattern.clone(),
            guard: case
                .guard
                .as_ref()
                .map(|guard| rewrite_placeholder_expr(guard, name)),
            body: match &case.body {
                MatchCaseBody::Expr(expr) => {
                    MatchCaseBody::Expr(rewrite_placeholder_expr(expr, name))
                }
                MatchCaseBody::Block(block) => MatchCaseBody::Block(rewrite_block(block, name)),
            },
            span: case.span,
        })
        .collect()
}

fn block_contains_placeholder(block: &Block) -> bool {
    block.statements.iter().any(stmt_contains_placeholder)
}

fn body_contains_placeholder(body: &Option<CallableBody>) -> bool {
    body.as_ref().is_some_and(|body| match body {
        CallableBody::Expr(expr) => contains_placeholder_expr(expr),
        CallableBody::Block(block) => block_contains_placeholder(block),
    })
}

fn stmt_contains_placeholder(stmt: &Stmt) -> bool {
    match stmt {
        Stmt::Binding(binding) => binding.values.iter().any(contains_placeholder_expr),
        Stmt::PatternBinding(stmt) => {
            stmt.clauses
                .iter()
                .any(|clause| contains_placeholder_expr(&clause.value))
                || contains_placeholder_expr(&stmt.value)
        }
        Stmt::Assignment(assignment) => {
            assignment.targets.iter().any(contains_placeholder_expr)
                || assignment.values.iter().any(contains_placeholder_expr)
        }
        Stmt::If(stmt) => {
            stmt.condition
                .as_ref()
                .is_some_and(contains_placeholder_expr)
                || stmt.condition_clauses.iter().any(|clause| match clause {
                    core::IfConditionClause::Let(clause) => {
                        contains_placeholder_expr(&clause.value)
                    }
                    core::IfConditionClause::Expr(condition) => {
                        contains_placeholder_expr(condition)
                    }
                })
                || stmt
                    .pattern_clauses
                    .iter()
                    .any(|clause| contains_placeholder_expr(&clause.value))
                || stmt
                    .pattern_value
                    .as_ref()
                    .is_some_and(contains_placeholder_expr)
                || stmt
                    .binding_value
                    .as_ref()
                    .is_some_and(contains_placeholder_expr)
                || block_contains_placeholder(&stmt.then_block)
                || stmt
                    .else_branch
                    .as_ref()
                    .is_some_and(|branch| match branch {
                        ElseBranch::If(stmt) => {
                            stmt_contains_placeholder(&Stmt::If((**stmt).clone()))
                        }
                        ElseBranch::Block(block) => block_contains_placeholder(block),
                    })
        }
        Stmt::Match(stmt) => {
            contains_placeholder_expr(&stmt.value)
                || stmt.cases.iter().any(|case| match &case.body {
                    MatchCaseBody::Expr(expr) => contains_placeholder_expr(expr),
                    MatchCaseBody::Block(block) => block_contains_placeholder(block),
                })
        }
        Stmt::While(stmt) => {
            contains_placeholder_expr(&stmt.condition) || block_contains_placeholder(&stmt.body)
        }
        Stmt::For(stmt) => {
            stmt.bindings.iter().any(|binding| {
                binding
                    .iterable
                    .as_ref()
                    .is_some_and(contains_placeholder_expr)
                    || binding.values.iter().any(contains_placeholder_expr)
            }) || block_contains_placeholder(&stmt.body)
        }
        Stmt::Return(stmt) => stmt.value.as_ref().is_some_and(contains_placeholder_expr),
        Stmt::Break(_) => false,
        Stmt::Continue(_) => false,
        Stmt::Expr(stmt) => contains_placeholder_expr(&stmt.expr),
        Stmt::LetElse(stmt) => {
            stmt.clauses
                .iter()
                .any(|clause| contains_placeholder_expr(&clause.value))
                || contains_placeholder_expr(&stmt.value)
                || block_contains_placeholder(&stmt.else_block)
        }
        Stmt::LocalFunction(function) => body_contains_placeholder(&Some(function.body.clone())),
    }
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
    fn lowers_local_functions_lambdas_record_updates_and_placeholders() {
        let program = parse_inline(
            r#"
            class Amount {
                amount Int
                description Str
            }

            def main() Int {
                base = 10
                inc (Int) -> Int = _ + 1
                plus = value Int -> value + base

                def add(value Int) Int = plus(value)

                current = Amount { 1, "a" }
                updated = current with { amount = add(inc(1)) }
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

    #[test]
    fn lowers_symbolic_binary_operators_as_method_calls() {
        let program = parse_inline(
            r#"
            class Vec {}

            impl Vec {
                def :+(value Int) Vec = this
                def :-(value Int) Vec = this
                def --(other Vec) Vec = this
            }

            def main() Unit {
                left Vec = Vec()
                right Vec = Vec()
                a = left :+ 1
                b = left :- 1
                c = left -- right
                d = List(1) ++ List(2)
            }
            "#,
        );

        let lowered = lower_program(&program);
        assert!(lowered.diagnostics.is_empty(), "{:#?}", lowered.diagnostics);
        let ir = lowered.program.expect("ir program");
        let main = ir.entry.and_then(|id| ir.function(id)).expect("main");
        let mut methods = Vec::new();
        for block in &main.blocks {
            for stmt in &block.statements {
                if let ir::StatementKind::Assign {
                    value:
                        ir::RValue::Call {
                            callee: ir::Callee::Method { method, .. },
                            ..
                        },
                    ..
                } = &stmt.kind
                {
                    methods.push(method.clone());
                }
            }
        }
        assert!(methods.iter().any(|method| method == ":+"), "{methods:#?}");
        assert!(methods.iter().any(|method| method == ":-"), "{methods:#?}");
        assert!(methods.iter().any(|method| method == "--"), "{methods:#?}");
        assert!(methods.iter().any(|method| method == "++"), "{methods:#?}");
    }
}
