use std::collections::HashMap;

use crate::{
    ast::{
        self, AssignOp, BinaryOp as AstBinaryOp, Block, CallableBody, ElseBranch, ElseExprBranch,
        Expr, FunctionDecl, ImplBlock, Item, MatchCaseBody, MethodDecl, Pattern, Stmt, TypeDecl,
        TypeMember, TypeRef,
    },
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
    fields: Vec<ImplicitField>,
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
struct ImplicitField {
    name: String,
    ty: ir::Type,
}

struct Lowerer<'a> {
    source: &'a ast::Program,
    diagnostics: Vec<Diagnostic>,
    program: ir::Program,
    type_ids: HashMap<String, ir::TypeId>,
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
            program: ir::Program::new(source.package.as_ref().map(|pkg| pkg.name.clone())),
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
                    if self.type_ids.contains_key(&decl.name) {
                        continue;
                    }
                    let mut ty = ir::TypeDef::new(decl.kind, decl.name.clone());
                    ty.visibility = decl.visibility;
                    ty.type_params = decl.type_params.iter().map(|param| param.name.clone()).collect();
                    ty.with_bounds = decl.with_bounds.iter().map(lower_type_ref).collect();
                    ty.span = Some(decl.span);
                    let id = self.program.add_type(ty);
                    self.type_ids.insert(decl.name.clone(), id);
                }
                Item::Function(function) => {
                    if self.function_ids.contains_key(&function.name) {
                        continue;
                    }
                    let id = self.declare_function(&function.name, function.visibility, &function.type_params, function.return_type.as_ref(), ir::FunctionKind::TopLevel, &function.params, None, function.span);
                    self.function_ids.insert(function.name.clone(), id);
                    self.function_work.push(FunctionWork {
                        id,
                        decl: function.clone(),
                    });
                }
                Item::Statement(Stmt::Binding(binding)) => {
                    for (index, local) in binding.bindings.iter().enumerate() {
                        if self.global_ids.contains_key(&local.name) || local.name == "_" {
                            continue;
                        }
                        let mut global = ir::Global::new(
                            local.name.clone(),
                            local.ty.as_ref().map(lower_type_ref).unwrap_or(ir::Type::Unknown),
                        );
                        global.visibility = binding.visibility;
                        global.mutable = local.mutable;
                        global.span = Some(local.span);
                        let id = self.program.add_global(global);
                        self.global_ids.insert(local.name.clone(), id);
                        if let Some(expr) = binding.values.get(index).cloned() {
                            self.global_inits.push(GlobalInit { id, expr });
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
        let Some(type_id) = self.type_ids.get(&decl.name).copied() else {
            return;
        };

        let mut fields = Vec::new();
        let mut implicit_fields = Vec::new();
        let mut methods_to_attach = Vec::new();
        let mut cases = Vec::new();

        for member in &decl.members {
            match member {
                TypeMember::Field(field) => {
                    let ty = field.ty.as_ref().map(lower_type_ref).unwrap_or(ir::Type::Unknown);
                    fields.push(ir::Field {
                        visibility: field.visibility,
                        mutable: field.mutable,
                        name: field.name.clone(),
                        ty: ty.clone(),
                        span: Some(field.span),
                    });
                    implicit_fields.push(ImplicitField {
                        name: field.name.clone(),
                        ty,
                    });
                }
                TypeMember::Method(method) => {
                    let (id, this_local) = self.declare_method_function(type_id, &decl.name, method);
                    methods_to_attach.push(id);
            self.method_work.push(MethodWork {
                id,
                decl: method.clone(),
                fields: implicit_fields.clone(),
                this_local,
            });
                }
                TypeMember::Case(case) => {
                    let field_names = case.fields.iter().map(|field| field.name.clone()).collect::<Vec<_>>();
                    self.case_fields
                        .insert(format!("{}.{}", decl.name, case.name), field_names);
                    let case_fields = case
                        .fields
                        .iter()
                        .map(|field| ir::Field {
                            visibility: field.visibility,
                            mutable: field.mutable,
                            name: field.name.clone(),
                            ty: field.ty.as_ref().map(lower_type_ref).unwrap_or(ir::Type::Unknown),
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
                "unsupported_impl_target",
                "lowering currently expects a named impl target",
                block.span,
            );
            return;
        };
        let Some(type_id) = self.type_ids.get(target_name).copied() else {
            return;
        };
        let fields = self
            .program
            .types
            .get(type_id.0)
            .map(|ty| {
                ty.fields
                    .iter()
                    .map(|field| ImplicitField {
                        name: field.name.clone(),
                        ty: field.ty.clone(),
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        let mut method_ids = Vec::new();
        for method in &block.methods {
            let (id, this_local) = self.declare_method_function(type_id, target_name, method);
            method_ids.push(id);
            self.method_work.push(MethodWork {
                id,
                decl: method.clone(),
                fields: fields.clone(),
                this_local,
            });
        }

        if let Some(ty) = self.program.types.get_mut(type_id.0) {
            ty.methods.extend(method_ids);
        }
    }

    fn declare_function(
        &mut self,
        name: &str,
        visibility: ast::Visibility,
        type_params: &[ast::TypeParam],
        return_type: Option<&TypeRef>,
        kind: ir::FunctionKind,
        params: &[ast::Param],
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
                param.ty.as_ref().map(lower_type_ref).unwrap_or(ir::Type::Unknown),
            );
        }
        self.program.add_function(function)
    }

    fn declare_method_function(
        &mut self,
        owner: ir::TypeId,
        owner_name: &str,
        method: &MethodDecl,
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
            let Some(function) = self.program.function_mut(job.id) else {
                continue;
            };
            let mut lowerer = FunctionLowerer::new(
                function,
                &self.global_ids,
                &self.function_ids,
                &self.case_fields,
                &mut self.diagnostics,
            );
            for (index, param) in job.decl.params.iter().enumerate() {
                if let Some(local_id) = lowerer.function.params.get(index).copied() {
                    lowerer.bind_existing(&param.name, local_id);
                }
            }
            lowerer.lower_callable_body(&job.decl.body, job.decl.span);
        }
    }

    fn lower_methods(&mut self) {
        let work = self.method_work.clone();
        for job in work {
            let Some(function) = self.program.function_mut(job.id) else {
                continue;
            };
            let mut lowerer = FunctionLowerer::new(
                function,
                &self.global_ids,
                &self.function_ids,
                &self.case_fields,
                &mut self.diagnostics,
            );
            lowerer.bind_existing("this", job.this_local);
            for field in &job.fields {
                lowerer.bind_implicit_field(&field.name, field.ty.clone());
            }
            for (index, param) in job.decl.params.iter().enumerate() {
                if let Some(local_id) = lowerer.function.params.get(index).copied() {
                    lowerer.bind_existing(&param.name, local_id);
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
        let jobs = self.global_inits.clone();
        for job in jobs {
            let Some(global) = self.program.globals.get_mut(job.id.0) else {
                continue;
            };
            if let Some(value) = lower_global_expr(&job.expr, &self.global_ids, &self.function_ids, &mut self.diagnostics) {
                global.initializer = Some(value);
            }
        }
    }

    fn add_error(&mut self, code: &'static str, message: impl Into<String>, span: Span) {
        self.diagnostics.push(Diagnostic::error(code, message, span));
    }
}

struct FunctionLowerer<'a> {
    function: &'a mut ir::Function,
    diagnostics: &'a mut Vec<Diagnostic>,
    globals: &'a HashMap<String, ir::GlobalId>,
    functions: &'a HashMap<String, ir::FunctionId>,
    case_fields: &'a HashMap<String, Vec<String>>,
    scopes: Vec<HashMap<String, ir::LocalId>>,
    implicit_fields: HashMap<String, ir::Type>,
    this_local: Option<ir::LocalId>,
    loop_exits: Vec<ir::BlockId>,
    current_block: Option<ir::BlockId>,
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

impl PatternPlan {
    fn always_true() -> Self {
        Self {
            condition: ir::Operand::Const(ir::Constant::Bool(true)),
            bindings: Vec::new(),
        }
    }
}

impl<'a> FunctionLowerer<'a> {
    fn new(
        function: &'a mut ir::Function,
        globals: &'a HashMap<String, ir::GlobalId>,
        functions: &'a HashMap<String, ir::FunctionId>,
        case_fields: &'a HashMap<String, Vec<String>>,
        diagnostics: &'a mut Vec<Diagnostic>,
    ) -> Self {
        let entry = function.entry;
        let mut this = Self {
            function,
            diagnostics,
            globals,
            functions,
            case_fields,
            scopes: vec![HashMap::new()],
            implicit_fields: HashMap::new(),
            this_local: None,
            loop_exits: Vec::new(),
            current_block: Some(entry),
        };
        if let Some(first) = this.function.locals.first() {
            if first.name == "this" {
                this.this_local = Some(first.id);
            }
        }
        this
    }

    fn bind_existing(&mut self, name: &str, local: ir::LocalId) {
        self.current_scope().insert(name.to_string(), local);
        if name == "this" {
            self.this_local = Some(local);
        }
    }

    fn bind_implicit_field(&mut self, name: &str, ty: ir::Type) {
        self.implicit_fields.insert(name.to_string(), ty);
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
                if let Stmt::Expr(expr_stmt) = stmt {
                    tail = Some(self.lower_expr(&expr_stmt.expr));
                    break;
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
                for (index, local) in binding.bindings.iter().enumerate() {
                    if local.name == "_" {
                        if let Some(expr) = binding.values.get(index) {
                            let value = self.lower_expr(expr);
                            self.push_statement(ir::Statement {
                                span: Some(expr.span()),
                                kind: ir::StatementKind::Eval {
                                    value: ir::RValue::Use(value),
                                },
                            });
                        }
                        continue;
                    }
                    let ty = local.ty.as_ref().map(lower_type_ref).unwrap_or(ir::Type::Unknown);
                    let local_id = self
                        .function
                        .add_local(local.name.clone(), ty, local.mutable, ir::LocalKind::Binding);
                    self.current_scope().insert(local.name.clone(), local_id);
                    if let Some(expr) = binding.values.get(index) {
                        let value = self.lower_expr(expr);
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
            Stmt::Assignment(assignment) => {
                for (target_expr, value_expr) in assignment.targets.iter().zip(assignment.values.iter()) {
                    let Some(target) = self.lower_place(target_expr) else {
                        continue;
                    };
                    let value = if assignment.operator == AssignOp::Reassign {
                        ir::RValue::Use(self.lower_expr(value_expr))
                    } else {
                        let Some(op) = map_assign_op(assignment.operator) else {
                            self.unsupported("unsupported assignment operator in lowering", assignment.span);
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
                    self.unsupported("break outside lowered loop", stmt.span);
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
            Stmt::Unwrap(stmt) => self.lower_unwrap_stmt(stmt, None),
            Stmt::UnwrapBlock(stmt) => self.lower_unwrap_block(stmt),
            Stmt::LocalFunction(function) => {
                self.unsupported(
                    format!("local function '{}' lowering is not implemented yet", function.name),
                    function.span,
                )
            }
        }
    }

    fn lower_if_stmt(&mut self, stmt: &ast::IfStmt) {
        let Some(condition) = stmt.condition.as_ref() else {
            self.unsupported("if unwrap lowering is not implemented yet", stmt.span);
            return;
        };
        let then_block = self.function.add_block();
        let else_block = self.function.add_block();
        let join_block = self.function.add_block();
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

    fn lower_else_branch(&mut self, branch: &ElseBranch) {
        match branch {
            ElseBranch::If(stmt) => self.lower_if_stmt(stmt),
            ElseBranch::Block(block) => {
                self.lower_block_statements(block);
            }
        }
    }

    fn lower_while_stmt(&mut self, stmt: &ast::WhileStmt) {
        let cond_block = self.function.add_block();
        let body_block = self.function.add_block();
        let exit_block = self.function.add_block();

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
        self.current_block = Some(body_block);
        self.lower_block_statements(&stmt.body);
        if self.current_block.is_some() {
            self.terminate(ir::Terminator::goto(cond_block));
        }
        self.loop_exits.pop();

        self.current_block = Some(exit_block);
    }

    fn lower_for_stmt(&mut self, stmt: &ast::ForStmt) {
        if stmt.bindings.is_empty() {
            let body_block = self.function.add_block();
            let exit_block = self.function.add_block();
            self.terminate(ir::Terminator::goto(body_block));
            self.loop_exits.push(exit_block);
            self.current_block = Some(body_block);
            self.lower_block_statements(&stmt.body);
            if self.current_block.is_some() {
                self.terminate(ir::Terminator::goto(body_block));
            }
            self.loop_exits.pop();
            self.current_block = Some(exit_block);
            return;
        }
        self.lower_for_bindings(&stmt.bindings, &|this| this.lower_block_statements(&stmt.body), stmt.span);
    }

    fn lower_for_yield_expr(
        &mut self,
        bindings: &[ast::ForBinding],
        yield_body: &Block,
        span: Span,
    ) -> ir::Operand {
        let result = self.function.add_temp(ir::Type::list(ir::Type::Unknown));
        self.push_statement(ir::Statement {
            span: Some(span),
            kind: ir::StatementKind::Assign {
                target: ir::Place::Local(result),
                value: ir::RValue::List(Vec::new()),
            },
        });

        if bindings.is_empty() {
            let body_block = self.function.add_block();
            let exit_block = self.function.add_block();
            self.terminate(ir::Terminator::goto(body_block));
            self.loop_exits.push(exit_block);
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
                    },
                },
            });
            if self.current_block.is_some() {
                self.terminate(ir::Terminator::goto(body_block));
            }
            self.loop_exits.pop();
            self.current_block = Some(exit_block);
            return ir::Operand::Copy(Box::new(ir::Place::Local(result)));
        }

        self.lower_for_bindings(
            bindings,
            &|this| {
                let yielded = this.lower_block_value(yield_body).unwrap_or(ir::Operand::Const(ir::Constant::Unit));
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
        bindings: &[ast::ForBinding],
        body: &dyn Fn(&mut Self),
        span: Span,
    ) {
        if bindings.is_empty() {
            body(self);
            return;
        }

        let first = &bindings[0];
        if !first.values.is_empty() && first.iterable.is_none() {
            for (index, binding) in first.bindings.iter().enumerate() {
                if binding.name == "_" {
                    continue;
                }
                let ty = binding.ty.as_ref().map(lower_type_ref).unwrap_or(ir::Type::Unknown);
                let local_id = self
                    .function
                    .add_local(binding.name.clone(), ty, binding.mutable, ir::LocalKind::Binding);
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
            self.lower_for_bindings(&bindings[1..], body, span);
            return;
        }

        let Some(iterable) = &first.iterable else {
            self.unsupported("for binding requires an iterable source", first.span);
            return;
        };

        let iter_value = self.lower_expr(iterable);
        let iter_local = self.function.add_temp(ir::Type::Unknown);
        self.push_statement(ir::Statement {
            span: Some(iterable.span()),
            kind: ir::StatementKind::Assign {
                target: ir::Place::Local(iter_local),
                value: ir::RValue::Call {
                    callee: ir::Callee::Intrinsic(ir::Intrinsic::IterInit),
                    args: vec![iter_value],
                },
            },
        });

        let cond_block = self.function.add_block();
        let body_block = self.function.add_block();
        let exit_block = self.function.add_block();
        self.terminate(ir::Terminator::goto(cond_block));

        self.current_block = Some(cond_block);
        let has_next = self.emit_temp_from_rvalue(
            ir::RValue::Call {
                callee: ir::Callee::Intrinsic(ir::Intrinsic::IterHasNext),
                args: vec![ir::Operand::Copy(Box::new(ir::Place::Local(iter_local)))],
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
        self.current_block = Some(body_block);
        self.push_scope();
        let item = self.emit_temp_from_rvalue(
            ir::RValue::Call {
                callee: ir::Callee::Intrinsic(ir::Intrinsic::IterNext),
                args: vec![ir::Operand::Copy(Box::new(ir::Place::Local(iter_local)))],
            },
            ir::Type::Unknown,
            Some(first.span),
        );
        self.bind_for_values(&first.bindings, item);
        self.lower_for_bindings(&bindings[1..], body, span);
        self.pop_scope();
        if self.current_block.is_some() {
            self.terminate(ir::Terminator::goto(cond_block));
        }
        self.loop_exits.pop();
        self.current_block = Some(exit_block);
    }

    fn bind_for_values(&mut self, bindings: &[ast::Binding], item: ir::Operand) {
        if bindings.len() <= 1 {
            if let Some(binding) = bindings.first() {
                self.bind_loop_binding(binding, item);
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
            self.bind_loop_binding(binding, field_value);
        }
    }

    fn bind_loop_binding(&mut self, binding: &ast::Binding, value: ir::Operand) {
        if binding.name == "_" {
            return;
        }
        let ty = binding.ty.as_ref().map(lower_type_ref).unwrap_or(ir::Type::Unknown);
        let local_id = self
            .function
            .add_local(binding.name.clone(), ty, binding.mutable, ir::LocalKind::Binding);
        self.current_scope().insert(binding.name.clone(), local_id);
        self.push_statement(ir::Statement {
            span: Some(binding.span),
            kind: ir::StatementKind::Assign {
                target: ir::Place::Local(local_id),
                value: ir::RValue::Use(value),
            },
        });
    }

    fn lower_unwrap_block(&mut self, stmt: &ast::UnwrapBlockStmt) {
        let failure_value = self.function.add_temp(ir::Type::Unknown);
        let fallback_block = self.function.add_block();
        let continue_block = self.function.add_block();

        for clause in &stmt.clauses {
            self.lower_unwrap_stmt(clause, Some((failure_value, fallback_block)));
            if self.current_block.is_none() {
                break;
            }
        }

        if self.current_block.is_some() {
            self.terminate(ir::Terminator::goto(continue_block));
        }

        self.current_block = Some(fallback_block);
        if let Some(else_block) = &stmt.else_block {
            let value = self
                .lower_block_value(else_block)
                .unwrap_or(ir::Operand::Const(ir::Constant::Unit));
            self.terminate(ir::Terminator::ret(Some(value)));
        } else {
            self.terminate(ir::Terminator::ret(Some(ir::Operand::Copy(Box::new(
                ir::Place::Local(failure_value),
            )))));
        }

        self.current_block = Some(continue_block);
    }

    fn lower_unwrap_stmt(
        &mut self,
        stmt: &ast::UnwrapStmt,
        shared_fallback: Option<(ir::LocalId, ir::BlockId)>,
    ) {
        let source = self.lower_expr(&stmt.value);
        let source_local = self.function.add_temp(ir::Type::Unknown);
        self.push_statement(ir::Statement {
            span: Some(stmt.value.span()),
            kind: ir::StatementKind::Assign {
                target: ir::Place::Local(source_local),
                value: ir::RValue::Use(source),
            },
        });

        let success_block = self.function.add_block();
        let failure_block = if let Some((_, target)) = shared_fallback {
            target
        } else {
            self.function.add_block()
        };
        let continue_block = self.function.add_block();

        let present = self.emit_temp_from_rvalue(
            ir::RValue::Call {
                callee: ir::Callee::Intrinsic(ir::Intrinsic::UnwrapPresent),
                args: vec![ir::Operand::Copy(Box::new(ir::Place::Local(source_local)))],
            },
            ir::Type::Bool,
            Some(stmt.span),
        );
        self.terminate(ir::Terminator {
            span: Some(stmt.span),
            kind: ir::TerminatorKind::Branch {
                condition: present,
                then_block: success_block,
                else_block: failure_block,
            },
        });

        self.current_block = Some(success_block);
        let inner = self.emit_temp_from_rvalue(
            ir::RValue::Call {
                callee: ir::Callee::Intrinsic(ir::Intrinsic::UnwrapValue),
                args: vec![ir::Operand::Copy(Box::new(ir::Place::Local(source_local)))],
            },
            ir::Type::Unknown,
            Some(stmt.span),
        );
        self.bind_unwrap_values(&stmt.bindings, inner);
        if self.current_block.is_some() {
            self.terminate(ir::Terminator::goto(continue_block));
        }

        if let Some((failure_local, fallback_target)) = shared_fallback {
            self.current_block = Some(failure_block);
            self.push_statement(ir::Statement {
                span: Some(stmt.span),
                kind: ir::StatementKind::Assign {
                    target: ir::Place::Local(failure_local),
                    value: ir::RValue::Use(ir::Operand::Copy(Box::new(ir::Place::Local(
                        source_local,
                    )))),
                },
            });
            self.terminate(ir::Terminator::goto(fallback_target));
        } else {
            self.current_block = Some(failure_block);
            if let Some(else_block) = &stmt.else_block {
                let value = self
                    .lower_block_value(else_block)
                    .unwrap_or(ir::Operand::Const(ir::Constant::Unit));
                self.terminate(ir::Terminator::ret(Some(value)));
            } else {
                self.terminate(ir::Terminator::ret(Some(ir::Operand::Copy(Box::new(
                    ir::Place::Local(source_local),
                )))));
            }
        }

        self.current_block = Some(continue_block);
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
        let ty = binding.ty.as_ref().map(lower_type_ref).unwrap_or(ir::Type::Unknown);
        let local_id = self
            .function
            .add_local(binding.name.clone(), ty, false, ir::LocalKind::Binding);
        self.current_scope().insert(binding.name.clone(), local_id);
        self.push_statement(ir::Statement {
            span: Some(binding.span),
            kind: ir::StatementKind::Assign {
                target: ir::Place::Local(local_id),
                value: ir::RValue::Use(value),
            },
        });
    }

    fn lower_match_stmt(&mut self, stmt: &ast::MatchStmt) {
        let scrutinee = self.lower_expr(&stmt.value);
        let join_block = self.function.add_block();
        self.current_block = self.current_block.or(Some(self.function.entry));

        for (index, case) in stmt.cases.iter().enumerate() {
            let body_block = self.function.add_block();
            let fail_block = if index + 1 == stmt.cases.len() {
                join_block
            } else {
                self.function.add_block()
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
        cases: &[ast::MatchCase],
        span: Span,
    ) -> ir::Operand {
        let scrutinee = self.lower_expr(value);
        let result = self.function.add_temp(if partial {
            ir::Type::option(ir::Type::Unknown)
        } else {
            ir::Type::Unknown
        });
        let join_block = self.function.add_block();
        self.current_block = self.current_block.or(Some(self.function.entry));

        for case in cases {
            let body_block = self.function.add_block();
            let fail_block = self.function.add_block();
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
        case: &ast::MatchCase,
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
            let guard_block = self.function.add_block();
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
                let right = self.lower_expr(value);
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
                PatternPlan { condition, bindings }
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
                let case_name = path.last().cloned().unwrap_or_default();
                let base_condition = self.emit_temp_from_rvalue(
                    ir::RValue::Call {
                        callee: ir::Callee::Intrinsic(ir::Intrinsic::VariantIs(case_name.clone())),
                        args: vec![scrutinee.clone()],
                    },
                    ir::Type::Bool,
                    Some(*span),
                );
                let mut conditions = vec![base_condition];
                let mut bindings = Vec::new();
                let field_names = self.lookup_case_fields(path, args.len());
                for (index, arg) in args.iter().enumerate() {
                    let field_name = field_names
                        .as_ref()
                        .and_then(|names| names.get(index).cloned())
                        .unwrap_or_else(|| format!("_{}", index + 1));
                    let field = self.emit_temp_from_rvalue(
                        ir::RValue::Call {
                            callee: ir::Callee::Intrinsic(ir::Intrinsic::VariantField(field_name)),
                            args: vec![scrutinee.clone()],
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

    fn lookup_case_fields(&self, path: &[String], arity: usize) -> Option<Vec<String>> {
        let case_name = path.last()?;
        if arity == 0 {
            return Some(Vec::new());
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
        None
    }

    fn apply_pending_bindings(&mut self, bindings: Vec<PendingBinding>) {
        for binding in bindings {
            let local_id = self
                .function
                .add_local(binding.name.clone(), binding.ty, false, ir::LocalKind::Binding);
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
        let temp = self.function.add_temp(ty);
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
            Expr::Identifier { name, span } => self.lookup_value(name).unwrap_or_else(|| {
                self.unsupported(format!("unknown value '{}' during lowering", name), *span);
                ir::Operand::Const(ir::Constant::Unit)
            }),
            Expr::Group { inner, .. } => self.lower_expr(inner),
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
            Expr::If {
                condition,
                then_block,
                else_branch,
                span,
            } => self.lower_if_expr(condition, then_block, else_branch, *span),
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
            _ => {
                let Some(rvalue) = self.lower_rvalue(expr) else {
                    self.unsupported("expression form is not implemented in lowering", expr.span());
                    return ir::Operand::Const(ir::Constant::Unit);
                };
                let temp = self.function.add_temp(ir::Type::Unknown);
                self.push_statement(ir::Statement {
                    span: Some(expr.span()),
                    kind: ir::StatementKind::Assign {
                        target: ir::Place::Local(temp),
                        value: rvalue,
                    },
                });
                ir::Operand::Copy(Box::new(ir::Place::Local(temp)))
            }
        }
    }

    fn lower_if_expr(
        &mut self,
        condition: &Expr,
        then_block: &Block,
        else_branch: &ElseExprBranch,
        span: Span,
    ) -> ir::Operand {
        let temp = self.function.add_temp(ir::Type::Unknown);
        let then_id = self.function.add_block();
        let else_id = self.function.add_block();
        let join_id = self.function.add_block();

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
        match expr {
            Expr::ListLiteral { items, .. } => Some(ir::RValue::List(
                items.iter().map(|item| self.lower_expr(item)).collect(),
            )),
            Expr::TupleLiteral { items, .. } => Some(ir::RValue::Tuple(
                items.iter().map(|item| self.lower_expr(item)).collect(),
            )),
            Expr::RecordLiteral { fields, .. } => Some(ir::RValue::Record(
                fields
                    .iter()
                    .map(|field| ir::NamedOperand {
                        name: field.name.clone().unwrap_or_default(),
                        value: self.lower_expr(&field.value),
                    })
                    .collect(),
            )),
            Expr::Unary { op, expr, .. } => Some(ir::RValue::Unary {
                op: match op {
                    ast::UnaryOp::Neg => ir::UnaryOp::Neg,
                    ast::UnaryOp::Not => ir::UnaryOp::Not,
                },
                operand: self.lower_expr(expr),
            }),
            Expr::Binary {
                left,
                op,
                right,
                span,
            } => Some(ir::RValue::Binary {
                op: map_binary_op(*op).unwrap_or_else(|| {
                    self.unsupported("binary operator lowering is not implemented yet", *span);
                    ir::BinaryOp::Add
                }),
                left: self.lower_expr(left),
                right: self.lower_expr(right),
            }),
            Expr::Call { callee, args, .. } => Some(ir::RValue::Call {
                callee: self.lower_callee(callee),
                args: args.iter().map(|arg| self.lower_expr(&arg.value)).collect(),
            }),
            Expr::Member { receiver, name, .. } => Some(ir::RValue::Field {
                base: self.lower_expr(receiver),
                name: name.clone(),
            }),
            Expr::Index { receiver, index, .. } => Some(ir::RValue::Index {
                base: self.lower_expr(receiver),
                index: self.lower_expr(index),
            }),
            Expr::Is { left, target, .. } => Some(ir::RValue::TypeTest {
                operand: self.lower_expr(left),
                ty: lower_type_ref(target),
            }),
            Expr::Lambda { span, .. } => {
                self.unsupported("lambda lowering is not implemented yet", *span);
                None
            }
            Expr::AnonymousInterface { span, .. } => {
                self.unsupported("anonymous interface lowering is not implemented yet", *span);
                None
            }
            Expr::RecordUpdate { span, .. } => {
                self.unsupported("record update lowering is not implemented yet", *span);
                None
            }
            Expr::Placeholder { span } => {
                self.unsupported("placeholder lowering is not implemented yet", *span);
                None
            }
            _ => None,
        }
    }

    fn lower_callee(&mut self, callee: &Expr) -> ir::Callee {
        if let Some(path) = expr_path(callee) {
            if path.len() == 1 {
                let name = &path[0];
                if let Some(intrinsic) = intrinsic_for_name(name) {
                    return ir::Callee::Intrinsic(intrinsic);
                }
                if let Some(function) = self.functions.get(name).copied() {
                    return ir::Callee::Direct(function);
                }
                if let Some(value) = self.lookup_value(name) {
                    return ir::Callee::Indirect(value);
                }
            }
            return ir::Callee::Named { path };
        }

        if let Expr::Member { receiver, name, .. } = callee {
            return ir::Callee::Method {
                receiver: self.lower_expr(receiver),
                method: name.clone(),
            };
        }

        ir::Callee::Indirect(self.lower_expr(callee))
    }

    fn lower_place(&mut self, expr: &Expr) -> Option<ir::Place> {
        match expr {
            Expr::Identifier { name, span } => self.lookup_place(name).or_else(|| {
                self.unsupported(format!("unknown assignment target '{}'", name), *span);
                None
            }),
            Expr::Member { receiver, name, .. } => Some(ir::Place::Field {
                base: Box::new(self.lower_expr(receiver)),
                name: name.clone(),
            }),
            Expr::Index { receiver, index, .. } => Some(ir::Place::Index {
                base: Box::new(self.lower_expr(receiver)),
                index: Box::new(self.lower_expr(index)),
            }),
            Expr::Group { inner, .. } => self.lower_place(inner),
            _ => {
                self.unsupported("invalid assignment target during lowering", expr.span());
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
        if let (Some(this_local), Some(_)) = (self.this_local, self.implicit_fields.get(name)) {
            return Some(ir::Place::Field {
                base: Box::new(ir::Operand::Copy(Box::new(ir::Place::Local(this_local)))),
                name: name.to_string(),
            });
        }
        None
    }

    fn current_block_mut(&mut self) -> Option<&mut ir::BasicBlock> {
        let current = self.current_block?;
        self.function.block_mut(current)
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
        self.function.blocks.iter().any(|block| match &block.terminator.kind {
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

    fn push_scope(&mut self) {
        self.scopes.push(HashMap::new());
    }

    fn pop_scope(&mut self) {
        self.scopes.pop();
    }

    fn unsupported(&mut self, message: impl Into<String>, span: Span) {
        self.diagnostics
            .push(Diagnostic::error("lower_unsupported", message, span));
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
        TypeRef::Tuple { fields, .. } => {
            ir::Type::Tuple(fields.iter().map(|field| lower_type_ref(&field.ty)).collect())
        }
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
        Expr::Group { inner, .. } => expr_path(inner),
        _ => None,
    }
}

fn lower_global_expr(
    expr: &Expr,
    globals: &HashMap<String, ir::GlobalId>,
    functions: &HashMap<String, ir::FunctionId>,
    diagnostics: &mut Vec<Diagnostic>,
) -> Option<ir::RValue> {
    match expr {
        Expr::Identifier { name, span } => {
            if let Some(global) = globals.get(name).copied() {
                Some(ir::RValue::Use(ir::Operand::Copy(Box::new(ir::Place::Global(global)))))
            } else {
                diagnostics.push(Diagnostic::error(
                    "lower_unsupported",
                    format!("unknown global value '{}' in initializer lowering", name),
                    *span,
                ));
                None
            }
        }
        Expr::Integer { raw, .. } => Some(ir::RValue::Use(ir::Operand::Const(
            ir::Constant::Int(raw.parse::<i64>().unwrap_or(0)),
        ))),
        Expr::Float { raw, .. } => Some(ir::RValue::Use(ir::Operand::Const(
            ir::Constant::Float(raw.parse::<f64>().unwrap_or(0.0)),
        ))),
        Expr::String { raw, .. } => Some(ir::RValue::Use(ir::Operand::Const(
            ir::Constant::String(raw.clone()),
        ))),
        Expr::Bool { value, .. } => Some(ir::RValue::Use(ir::Operand::Const(
            ir::Constant::Bool(*value),
        ))),
        Expr::Unit { .. } => Some(ir::RValue::Use(ir::Operand::Const(ir::Constant::Unit))),
        Expr::Group { inner, .. } => lower_global_expr(inner, globals, functions, diagnostics),
        Expr::Unary { op, expr, .. } => Some(ir::RValue::Unary {
            op: match op {
                ast::UnaryOp::Neg => ir::UnaryOp::Neg,
                ast::UnaryOp::Not => ir::UnaryOp::Not,
            },
            operand: lower_global_operand(expr, globals, diagnostics)?,
        }),
        Expr::Binary {
            left,
            op,
            right,
            span,
        } => Some(ir::RValue::Binary {
            op: map_binary_op(*op).unwrap_or_else(|| {
                diagnostics.push(Diagnostic::error(
                    "lower_unsupported",
                    "binary operator is not supported in global initializer lowering",
                    *span,
                ));
                ir::BinaryOp::Add
            }),
            left: lower_global_operand(left, globals, diagnostics)?,
            right: lower_global_operand(right, globals, diagnostics)?,
        }),
        Expr::ListLiteral { items, .. } => Some(ir::RValue::List(
            items
                .iter()
                .map(|item| lower_global_operand(item, globals, diagnostics))
                .collect::<Option<Vec<_>>>()?,
        )),
        Expr::TupleLiteral { items, .. } => Some(ir::RValue::Tuple(
            items
                .iter()
                .map(|item| lower_global_operand(item, globals, diagnostics))
                .collect::<Option<Vec<_>>>()?,
        )),
        Expr::Call { callee, args, .. } => Some(ir::RValue::Call {
            callee: lower_global_callee(callee, globals, functions, diagnostics),
            args: args
                .iter()
                .map(|arg| lower_global_operand(&arg.value, globals, diagnostics))
                .collect::<Option<Vec<_>>>()?,
        }),
        Expr::Member { receiver, name, .. } => Some(ir::RValue::Field {
            base: lower_global_operand(receiver, globals, diagnostics)?,
            name: name.clone(),
        }),
        Expr::Index { receiver, index, .. } => Some(ir::RValue::Index {
            base: lower_global_operand(receiver, globals, diagnostics)?,
            index: lower_global_operand(index, globals, diagnostics)?,
        }),
        Expr::Is { left, target, .. } => Some(ir::RValue::TypeTest {
            operand: lower_global_operand(left, globals, diagnostics)?,
            ty: lower_type_ref(target),
        }),
        other => {
            diagnostics.push(Diagnostic::error(
                "lower_unsupported",
                "global initializer expression is not implemented in lowering",
                other.span(),
            ));
            None
        }
    }
}

fn lower_global_operand(
    expr: &Expr,
    globals: &HashMap<String, ir::GlobalId>,
    diagnostics: &mut Vec<Diagnostic>,
) -> Option<ir::Operand> {
    match expr {
        Expr::Identifier { name, span } => globals
            .get(name)
            .copied()
            .map(|global| ir::Operand::Copy(Box::new(ir::Place::Global(global))))
            .or_else(|| {
                diagnostics.push(Diagnostic::error(
                    "lower_unsupported",
                    format!("unknown global value '{}' in initializer lowering", name),
                    *span,
                ));
                None
            }),
        Expr::Integer { raw, .. } => Some(ir::Operand::Const(ir::Constant::Int(
            raw.parse::<i64>().unwrap_or(0),
        ))),
        Expr::Float { raw, .. } => Some(ir::Operand::Const(ir::Constant::Float(
            raw.parse::<f64>().unwrap_or(0.0),
        ))),
        Expr::String { raw, .. } => Some(ir::Operand::Const(ir::Constant::String(raw.clone()))),
        Expr::Bool { value, .. } => Some(ir::Operand::Const(ir::Constant::Bool(*value))),
        Expr::Unit { .. } => Some(ir::Operand::Const(ir::Constant::Unit)),
        Expr::Group { inner, .. } => lower_global_operand(inner, globals, diagnostics),
        _ => {
            diagnostics.push(Diagnostic::error(
                "lower_unsupported",
                "complex operand in global initializer lowering needs temps, which are not implemented yet",
                expr.span(),
            ));
            None
        }
    }
}

fn lower_global_callee(
    callee: &Expr,
    globals: &HashMap<String, ir::GlobalId>,
    functions: &HashMap<String, ir::FunctionId>,
    diagnostics: &mut Vec<Diagnostic>,
) -> ir::Callee {
    if let Some(path) = expr_path(callee) {
        if path.len() == 1 {
            let name = &path[0];
            if let Some(intrinsic) = intrinsic_for_name(name) {
                return ir::Callee::Intrinsic(intrinsic);
            }
            if let Some(function) = functions.get(name).copied() {
                return ir::Callee::Direct(function);
            }
            if let Some(global) = globals.get(name).copied() {
                return ir::Callee::Indirect(ir::Operand::Copy(Box::new(ir::Place::Global(global))));
            }
        }
        return ir::Callee::Named { path };
    }

    if let Expr::Member { receiver, name, .. } = callee {
        if let Some(base) = lower_global_operand(receiver, globals, diagnostics) {
            return ir::Callee::Method {
                receiver: base,
                method: name.clone(),
            };
        }
    }

    diagnostics.push(Diagnostic::error(
        "lower_unsupported",
        "global initializer callee is not implemented in lowering",
        callee.span(),
    ));
    ir::Callee::Named {
        path: vec!["<error>".to_string()],
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
        assert!(main.blocks.iter().any(|block| matches!(
            block.terminator.kind,
            ir::TerminatorKind::Branch { .. }
        )));
    }

    #[test]
    fn lowers_types_and_methods() {
        let program = parse_inline(
            r#"
            class Counter {
                value Int
            }

            impl Counter {
                def bump(delta Int) Int = value + delta
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
    fn lowers_match_unwrap_and_for_forms() {
        let program = parse_inline(
            r#"
            def main() Int {
                total Int = 0
                unwrap item <- Some(3)
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
        assert!(main.blocks.iter().any(|block| matches!(
            block.terminator.kind,
            ir::TerminatorKind::Branch { .. }
        )));
        assert!(main.blocks.iter().any(|block| {
            block.statements.iter().any(|stmt| matches!(
                stmt.kind,
                ir::StatementKind::Assign {
                    value: ir::RValue::Call {
                        callee: ir::Callee::Intrinsic(ir::Intrinsic::UnwrapPresent),
                        ..
                    },
                    ..
                }
            ))
        }));
        assert!(main.blocks.iter().any(|block| {
            block.statements.iter().any(|stmt| matches!(
                stmt.kind,
                ir::StatementKind::Assign {
                    value: ir::RValue::Call {
                        callee: ir::Callee::Intrinsic(ir::Intrinsic::IterHasNext),
                        ..
                    },
                    ..
                }
            ))
        }));
    }
}
