use std::{
    cell::RefCell,
    collections::HashSet,
    fmt,
    path::{Path, PathBuf},
    rc::Rc,
};

use crate::{
    ast,
    diagnostic::Diagnostic,
    ir,
    lower::lower_program,
    resolver::{LoadedModule, LocatedDiagnostic, ModuleGraph, load_module_graph},
    runtime,
    source::{LineColumn, Span},
    typecheck::check_path,
};

#[derive(Debug, Clone, Default)]
pub struct RunResult {
    pub diagnostics: Vec<Diagnostic>,
    pub output: String,
    pub return_value: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct PathRunResult {
    pub diagnostics: Vec<LocatedDiagnostic>,
    pub output: String,
    pub return_value: Option<String>,
}

pub fn run_program(program: &ir::Program) -> RunResult {
    run_program_entry(program, None)
}

pub fn run_program_entry(program: &ir::Program, requested_entry: Option<&str>) -> RunResult {
    let mut interpreter = Interpreter::new(program);
    match interpreter.run(requested_entry) {
        Ok(Some(value)) => RunResult {
            diagnostics: Vec::new(),
            output: interpreter.output,
            return_value: Some(value.render()),
        },
        Ok(None) => RunResult {
            diagnostics: Vec::new(),
            output: interpreter.output,
            return_value: None,
        },
        Err(diagnostic) => RunResult {
            diagnostics: vec![diagnostic],
            output: interpreter.output,
            return_value: None,
        },
    }
}

pub fn run_path(
    path: impl AsRef<Path>,
    requested_entry: Option<&str>,
) -> Result<PathRunResult, String> {
    let path = path.as_ref();
    let checked = check_path(path)?;
    if !checked.diagnostics.is_empty() {
        return Ok(PathRunResult {
            diagnostics: checked.diagnostics,
            output: String::new(),
            return_value: None,
        });
    }

    let (graph, root_path) = load_module_graph(path)?;
    let root_module = graph
        .modules
        .get(&root_path)
        .ok_or_else(|| format!("loaded root module missing {}", root_path.display()))?;
    let program = merged_runtime_program(&graph, &root_path)?;

    let lowered = lower_program(&program);
    if !lowered.diagnostics.is_empty() {
        return Ok(PathRunResult {
            diagnostics: lowered
                .diagnostics
                .into_iter()
                .map(|diagnostic| LocatedDiagnostic {
                    path: root_module.display_path.clone(),
                    diagnostic,
                })
                .collect(),
            output: String::new(),
            return_value: None,
        });
    }

    let lowered_program = lowered
        .program
        .expect("ir program after successful lowering");
    let run = run_program_entry(&lowered_program, requested_entry);
    Ok(PathRunResult {
        diagnostics: run
            .diagnostics
            .into_iter()
            .map(|diagnostic| LocatedDiagnostic {
                path: root_module.display_path.clone(),
                diagnostic,
            })
            .collect(),
        output: run.output,
        return_value: run.return_value,
    })
}

pub(crate) fn merged_runtime_program(
    graph: &ModuleGraph,
    root: &PathBuf,
) -> Result<ast::Program, String> {
    let mut order = Vec::new();
    let mut seen = HashSet::new();
    collect_runtime_module_order(graph, root, &mut seen, &mut order);

    let root_module = graph
        .modules
        .get(root)
        .ok_or_else(|| format!("loaded root module missing {}", root.display()))?;

    let mut merged = ast::Program {
        module: root_module.program.module.clone(),
        imports: Vec::new(),
        items: Vec::new(),
        span: root_module.program.span,
    };

    merged
        .items
        .extend(prepare_runtime_module(root_module, graph, true).items);
    for path in order {
        if &path == root {
            continue;
        }
        let Some(module) = graph.modules.get(&path) else {
            continue;
        };
        merged
            .items
            .extend(prepare_runtime_module(module, graph, false).items);
    }

    Ok(merged)
}

fn collect_runtime_module_order(
    graph: &ModuleGraph,
    root: &PathBuf,
    seen: &mut HashSet<PathBuf>,
    out: &mut Vec<PathBuf>,
) {
    if !seen.insert(root.clone()) {
        return;
    }
    let Some(module) = graph.modules.get(root) else {
        return;
    };
    for dependency in &module.dependencies {
        collect_runtime_module_order(graph, dependency, seen, out);
    }
    out.push(root.clone());
}

fn prepare_runtime_module(
    module: &LoadedModule,
    graph: &ModuleGraph,
    is_root: bool,
) -> ast::Program {
    let mut program = module.program.clone();
    rewrite_program_for_runtime(&mut program, module, graph);
    program.imports.clear();
    if module.source == crate::resolver::ModuleSource::Library {
        program.items.retain(|item| match item {
            ast::Item::Type(decl) => !module.typecheck_only_types.contains(&decl.name),
            _ => true,
        });
    }
    if !is_root || module.source == crate::resolver::ModuleSource::Library {
        program.items.retain(
            |item| !matches!(item, ast::Item::Function(function) if function.name == "main"),
        );
    }
    program
}

fn rewrite_program_for_runtime(
    program: &mut ast::Program,
    module: &LoadedModule,
    graph: &ModuleGraph,
) {
    for item in &mut program.items {
        rewrite_item_for_runtime(item, module, graph);
    }
}

fn rewrite_item_for_runtime(item: &mut ast::Item, module: &LoadedModule, graph: &ModuleGraph) {
    match item {
        ast::Item::Function(function) => rewrite_function_for_runtime(function, module, graph),
        ast::Item::Type(decl) => rewrite_type_decl_for_runtime(decl, module, graph),
        ast::Item::Impl(block) => rewrite_impl_block_for_runtime(block, module, graph),
        ast::Item::Statement(stmt) => rewrite_stmt_for_runtime(stmt, module, graph),
    }
}

fn rewrite_type_decl_for_runtime(
    decl: &mut ast::TypeDecl,
    module: &LoadedModule,
    graph: &ModuleGraph,
) {
    for bound in &mut decl.with_bounds {
        rewrite_type_ref_for_runtime(bound, module);
    }
    for member in &mut decl.members {
        match member {
            ast::TypeMember::Field(field) => {
                if let Some(ty) = &mut field.ty {
                    rewrite_type_ref_for_runtime(ty, module);
                }
                if let Some(initializer) = &mut field.initializer {
                    rewrite_expr_for_runtime(initializer, module, graph);
                }
            }
            ast::TypeMember::Method(method) => rewrite_method_for_runtime(method, module, graph),
            ast::TypeMember::Case(case) => {
                for field in &mut case.fields {
                    if let Some(ty) = &mut field.ty {
                        rewrite_type_ref_for_runtime(ty, module);
                    }
                    if let Some(initializer) = &mut field.initializer {
                        rewrite_expr_for_runtime(initializer, module, graph);
                    }
                }
            }
        }
    }
}

fn rewrite_impl_block_for_runtime(
    block: &mut ast::ImplBlock,
    module: &LoadedModule,
    graph: &ModuleGraph,
) {
    rewrite_type_ref_for_runtime(&mut block.target, module);
    for method in &mut block.methods {
        rewrite_method_for_runtime(method, module, graph);
    }
}

fn rewrite_function_for_runtime(
    function: &mut ast::FunctionDecl,
    module: &LoadedModule,
    graph: &ModuleGraph,
) {
    for param in &mut function.params {
        if let Some(ty) = &mut param.ty {
            rewrite_type_ref_for_runtime(ty, module);
        }
    }
    if let Some(ret) = &mut function.return_type {
        rewrite_type_ref_for_runtime(ret, module);
    }
    rewrite_callable_body_for_runtime(&mut function.body, module, graph);
}

fn rewrite_method_for_runtime(
    method: &mut ast::MethodDecl,
    module: &LoadedModule,
    graph: &ModuleGraph,
) {
    for param in &mut method.params {
        if let Some(ty) = &mut param.ty {
            rewrite_type_ref_for_runtime(ty, module);
        }
    }
    if let Some(ret) = &mut method.return_type {
        rewrite_type_ref_for_runtime(ret, module);
    }
    if let Some(body) = &mut method.body {
        rewrite_callable_body_for_runtime(body, module, graph);
    }
}

fn rewrite_callable_body_for_runtime(
    body: &mut ast::CallableBody,
    module: &LoadedModule,
    graph: &ModuleGraph,
) {
    match body {
        ast::CallableBody::Block(block) => rewrite_block_for_runtime(block, module, graph),
        ast::CallableBody::Expr(expr) => rewrite_expr_for_runtime(expr, module, graph),
    }
}

fn rewrite_block_for_runtime(block: &mut ast::Block, module: &LoadedModule, graph: &ModuleGraph) {
    for stmt in &mut block.statements {
        rewrite_stmt_for_runtime(stmt, module, graph);
    }
}

fn rewrite_stmt_for_runtime(stmt: &mut ast::Stmt, module: &LoadedModule, graph: &ModuleGraph) {
    match stmt {
        ast::Stmt::Binding(binding) => {
            for local in &mut binding.bindings {
                if let Some(ty) = &mut local.ty {
                    rewrite_type_ref_for_runtime(ty, module);
                }
            }
            for value in &mut binding.values {
                rewrite_expr_for_runtime(value, module, graph);
            }
        }
        ast::Stmt::PatternBinding(stmt) => {
            for clause in &mut stmt.clauses {
                rewrite_pattern_for_runtime(&mut clause.pattern, module);
                rewrite_expr_for_runtime(&mut clause.value, module, graph);
            }
            rewrite_pattern_for_runtime(&mut stmt.pattern, module);
            rewrite_expr_for_runtime(&mut stmt.value, module, graph);
        }
        ast::Stmt::Assignment(assign) => {
            for target in &mut assign.targets {
                rewrite_expr_for_runtime(target, module, graph);
            }
            for value in &mut assign.values {
                rewrite_expr_for_runtime(value, module, graph);
            }
        }
        ast::Stmt::Defer(stmt) => match &mut stmt.action {
            ast::DeferAction::Call(expr) => rewrite_expr_for_runtime(expr, module, graph),
            ast::DeferAction::Block(block) => rewrite_block_for_runtime(block, module, graph),
        },
        ast::Stmt::LetElse(stmt) => {
            for clause in &mut stmt.clauses {
                rewrite_pattern_for_runtime(&mut clause.pattern, module);
                rewrite_expr_for_runtime(&mut clause.value, module, graph);
            }
            rewrite_pattern_for_runtime(&mut stmt.pattern, module);
            rewrite_expr_for_runtime(&mut stmt.value, module, graph);
            rewrite_block_for_runtime(&mut stmt.else_block, module, graph);
        }
        ast::Stmt::If(stmt) => rewrite_if_stmt_for_runtime(stmt, module, graph),
        ast::Stmt::Match(stmt) => {
            rewrite_expr_for_runtime(&mut stmt.value, module, graph);
            for case in &mut stmt.cases {
                rewrite_match_case_for_runtime(case, module, graph);
            }
        }
        ast::Stmt::While(stmt) => {
            rewrite_expr_for_runtime(&mut stmt.condition, module, graph);
            rewrite_block_for_runtime(&mut stmt.body, module, graph);
        }
        ast::Stmt::For(stmt) => {
            for binding in &mut stmt.bindings {
                rewrite_for_binding_for_runtime(binding, module, graph);
            }
            rewrite_block_for_runtime(&mut stmt.body, module, graph);
        }
        ast::Stmt::Return(stmt) => {
            if let Some(value) = &mut stmt.value {
                rewrite_expr_for_runtime(value, module, graph);
            }
        }
        ast::Stmt::Break(_) => {}
        ast::Stmt::Continue(_) => {}
        ast::Stmt::Expr(stmt) => rewrite_expr_for_runtime(&mut stmt.expr, module, graph),
        ast::Stmt::LocalFunction(function) => rewrite_function_for_runtime(function, module, graph),
    }
}

fn rewrite_if_stmt_for_runtime(stmt: &mut ast::IfStmt, module: &LoadedModule, graph: &ModuleGraph) {
    if let Some(condition) = &mut stmt.condition {
        rewrite_expr_for_runtime(condition, module, graph);
    }
    for clause in &mut stmt.condition_clauses {
        match clause {
            ast::IfConditionClause::Let(clause) => {
                rewrite_pattern_for_runtime(&mut clause.pattern, module);
                rewrite_expr_for_runtime(&mut clause.value, module, graph);
            }
            ast::IfConditionClause::Expr(condition) => {
                rewrite_expr_for_runtime(condition, module, graph);
            }
        }
    }
    for clause in &mut stmt.pattern_clauses {
        rewrite_pattern_for_runtime(&mut clause.pattern, module);
        rewrite_expr_for_runtime(&mut clause.value, module, graph);
    }
    if let Some(pattern) = &mut stmt.pattern {
        rewrite_pattern_for_runtime(pattern, module);
    }
    if let Some(value) = &mut stmt.pattern_value {
        rewrite_expr_for_runtime(value, module, graph);
    }
    for binding in &mut stmt.bindings {
        if let Some(ty) = &mut binding.ty {
            rewrite_type_ref_for_runtime(ty, module);
        }
    }
    if let Some(value) = &mut stmt.binding_value {
        rewrite_expr_for_runtime(value, module, graph);
    }
    rewrite_block_for_runtime(&mut stmt.then_block, module, graph);
    if let Some(branch) = &mut stmt.else_branch {
        rewrite_else_branch_for_runtime(branch, module, graph);
    }
}

fn rewrite_for_binding_for_runtime(
    binding: &mut ast::ForBinding,
    module: &LoadedModule,
    graph: &ModuleGraph,
) {
    if let Some(pattern) = &mut binding.pattern {
        rewrite_pattern_for_runtime(pattern, module);
    }
    for local in &mut binding.bindings {
        if let Some(ty) = &mut local.ty {
            rewrite_type_ref_for_runtime(ty, module);
        }
    }
    if let Some(iterable) = &mut binding.iterable {
        rewrite_expr_for_runtime(iterable, module, graph);
    }
    for value in &mut binding.values {
        rewrite_expr_for_runtime(value, module, graph);
    }
}

fn rewrite_else_branch_for_runtime(
    branch: &mut ast::ElseBranch,
    module: &LoadedModule,
    graph: &ModuleGraph,
) {
    match branch {
        ast::ElseBranch::If(stmt) => rewrite_if_stmt_for_runtime(stmt.as_mut(), module, graph),
        ast::ElseBranch::Block(block) => rewrite_block_for_runtime(block, module, graph),
    }
}

fn rewrite_match_case_for_runtime(
    case: &mut ast::MatchCase,
    module: &LoadedModule,
    graph: &ModuleGraph,
) {
    rewrite_pattern_for_runtime(&mut case.pattern, module);
    if let Some(guard) = &mut case.guard {
        rewrite_expr_for_runtime(guard, module, graph);
    }
    match &mut case.body {
        ast::MatchCaseBody::Block(block) => rewrite_block_for_runtime(block, module, graph),
        ast::MatchCaseBody::Expr(expr) => rewrite_expr_for_runtime(expr, module, graph),
    }
}

fn rewrite_pattern_for_runtime(pattern: &mut ast::Pattern, module: &LoadedModule) {
    match pattern {
        ast::Pattern::Wildcard { .. } | ast::Pattern::Binding { .. } => {}
        ast::Pattern::Extract { inner, .. } => rewrite_pattern_for_runtime(inner, module),
        ast::Pattern::Type { target, .. } => rewrite_type_ref_for_runtime(target, module),
        ast::Pattern::Literal { value, .. } => {
            // handled by parent expr rewrite when embedded in match cases
            let _ = value;
        }
        ast::Pattern::Tuple { elements, .. } => {
            for element in elements {
                rewrite_pattern_for_runtime(element, module);
            }
        }
        ast::Pattern::Constructor { path, args, .. } => {
            rewrite_pattern_path_for_runtime(path, module);
            for arg in args {
                rewrite_pattern_for_runtime(arg, module);
            }
        }
    }
}

fn rewrite_expr_for_runtime(expr: &mut ast::Expr, module: &LoadedModule, graph: &ModuleGraph) {
    match expr {
        ast::Expr::Identifier { name, span } => {
            if let Some(path) = rewritten_imported_symbol_path(module, name) {
                *expr = expr_from_path(path, *span);
            }
        }
        ast::Expr::Placeholder { .. }
        | ast::Expr::Integer { .. }
        | ast::Expr::Float { .. }
        | ast::Expr::String { .. }
        | ast::Expr::Bool { .. }
        | ast::Expr::Unit { .. } => {}
        ast::Expr::ListLiteral { items, .. } | ast::Expr::TupleLiteral { items, .. } => {
            for item in items {
                rewrite_expr_for_runtime(item, module, graph);
            }
        }
        ast::Expr::Call {
            callee,
            args,
            uses_brace_syntax: _,
            span,
        } => {
            rewrite_expr_for_runtime(callee, module, graph);
            for arg in args {
                rewrite_expr_for_runtime(&mut arg.value, module, graph);
            }
            if let Some(path) = rewritten_expr_path(module, callee) {
                *callee = Box::new(expr_from_path(path, *span));
            }
        }
        ast::Expr::Member {
            receiver,
            name,
            span,
        } => {
            rewrite_expr_for_runtime(receiver, module, graph);
            let mut current_path = match expr_path_ast(receiver) {
                Some(path) => path,
                None => return,
            };
            current_path.push(name.clone());
            if let Some(path) = rewritten_path_segments(module, &current_path) {
                *expr = expr_from_path(path, *span);
            }
        }
        ast::Expr::Index {
            receiver, index, ..
        } => {
            rewrite_expr_for_runtime(receiver, module, graph);
            rewrite_expr_for_runtime(index, module, graph);
        }
        ast::Expr::RecordUpdate {
            receiver, patch, ..
        } => {
            rewrite_expr_for_runtime(receiver, module, graph);
            rewrite_expr_for_runtime(patch, module, graph);
        }
        ast::Expr::RecordLiteral { fields, values, .. } => {
            for field in fields {
                rewrite_expr_for_runtime(&mut field.value, module, graph);
            }
            for value in values {
                rewrite_expr_for_runtime(value, module, graph);
            }
        }
        ast::Expr::AnonymousInterface {
            interfaces,
            methods,
            ..
        } => {
            for interface in interfaces {
                rewrite_type_ref_for_runtime(interface, module);
            }
            for method in methods {
                rewrite_method_for_runtime(method, module, graph);
            }
        }
        ast::Expr::Unary { expr: inner, .. } => rewrite_expr_for_runtime(inner, module, graph),
        ast::Expr::Try { value, handler, .. } => {
            rewrite_expr_for_runtime(value, module, graph);
            if let Some(handler) = handler {
                rewrite_expr_for_runtime(&mut handler.body, module, graph);
            }
        }
        ast::Expr::Lift { value, .. } => rewrite_expr_for_runtime(value, module, graph),
        ast::Expr::Binary { left, right, .. } => {
            rewrite_expr_for_runtime(left, module, graph);
            rewrite_expr_for_runtime(right, module, graph);
        }
        ast::Expr::Is { left, target, .. } => {
            rewrite_expr_for_runtime(left, module, graph);
            rewrite_type_ref_for_runtime(target, module);
        }
        ast::Expr::TypeOf { ty, .. } => {
            rewrite_type_ref_for_runtime(ty, module);
        }
        ast::Expr::If {
            condition,
            then_block,
            else_branch,
            ..
        } => {
            rewrite_expr_for_runtime(condition, module, graph);
            rewrite_block_for_runtime(then_block, module, graph);
            rewrite_else_expr_branch_for_runtime(else_branch, module, graph);
        }
        ast::Expr::Block { body, .. } => rewrite_block_for_runtime(body, module, graph),
        ast::Expr::Match { value, cases, .. } => {
            rewrite_expr_for_runtime(value, module, graph);
            for case in cases {
                rewrite_match_case_for_runtime(case, module, graph);
            }
        }
        ast::Expr::ForYield {
            bindings,
            yield_body,
            ..
        } => {
            for binding in bindings {
                rewrite_for_binding_for_runtime(binding, module, graph);
            }
            rewrite_block_for_runtime(yield_body, module, graph);
        }
        ast::Expr::Lambda { params, body, .. } => {
            for param in params {
                if let Some(ty) = &mut param.ty {
                    rewrite_type_ref_for_runtime(ty, module);
                }
                if let Some(destructure) = &mut param.destructure {
                    for binding in &mut destructure.bindings {
                        if let Some(ty) = &mut binding.ty {
                            rewrite_type_ref_for_runtime(ty, module);
                        }
                    }
                }
            }
            match body {
                ast::LambdaBody::Expr(expr) => rewrite_expr_for_runtime(expr, module, graph),
                ast::LambdaBody::Block(block) => rewrite_block_for_runtime(block, module, graph),
            }
        }
        ast::Expr::LiftedChain { base, segments, .. } => {
            rewrite_expr_for_runtime(base, module, graph);
            for segment in segments {
                rewrite_expr_for_runtime(&mut segment.body, module, graph);
            }
        }
        ast::Expr::Group { inner, .. } => rewrite_expr_for_runtime(inner, module, graph),
    }
}

fn rewrite_else_expr_branch_for_runtime(
    branch: &mut ast::ElseExprBranch,
    module: &LoadedModule,
    graph: &ModuleGraph,
) {
    match branch {
        ast::ElseExprBranch::If(expr) => rewrite_expr_for_runtime(expr, module, graph),
        ast::ElseExprBranch::Block(block) => rewrite_block_for_runtime(block, module, graph),
    }
}

fn rewrite_type_ref_for_runtime(reference: &mut ast::TypeRef, module: &LoadedModule) {
    match reference {
        ast::TypeRef::Wildcard { .. } => {}
        ast::TypeRef::Named { name, args, .. } => {
            if let Some(path) = rewritten_imported_symbol_path(module, name) {
                if path.len() == 1 {
                    *name = path[0].clone();
                }
            }
            for arg in args {
                rewrite_type_ref_for_runtime(arg, module);
            }
        }
        ast::TypeRef::Tuple { fields, .. } => {
            for field in fields {
                rewrite_type_ref_for_runtime(&mut field.ty, module);
            }
        }
        ast::TypeRef::Record { fields, .. } => {
            for field in fields {
                rewrite_type_ref_for_runtime(&mut field.ty, module);
            }
        }
        ast::TypeRef::Function { params, ret, .. } => {
            for param in params {
                rewrite_type_ref_for_runtime(param, module);
            }
            rewrite_type_ref_for_runtime(ret, module);
        }
    }
}

fn rewrite_pattern_path_for_runtime(path: &mut Vec<String>, module: &LoadedModule) {
    if path.is_empty() {
        return;
    }
    if let Some(rewritten) = rewritten_path_segments(module, path) {
        *path = rewritten;
    }
}

fn rewritten_expr_path(module: &LoadedModule, expr: &ast::Expr) -> Option<Vec<String>> {
    let path = expr_path_ast(expr)?;
    rewritten_path_segments(module, &path)
}

fn rewritten_path_segments(module: &LoadedModule, path: &[String]) -> Option<Vec<String>> {
    if path.is_empty() {
        return None;
    }
    if let Some(symbol) = module.symbol_imports.get(&path[0]) {
        let mut target = imported_symbol_path(symbol);
        target.extend(path.iter().skip(1).cloned());
        return Some(target);
    }
    if module.imports.contains_key(&path[0]) && path.len() > 1 {
        return Some(path[1..].to_vec());
    }
    None
}

fn rewritten_imported_symbol_path(module: &LoadedModule, name: &str) -> Option<Vec<String>> {
    module.symbol_imports.get(name).map(imported_symbol_path)
}

fn imported_symbol_path(symbol: &crate::resolver::ImportedSymbol) -> Vec<String> {
    if let Some(single_name) = &symbol.single_name {
        vec![single_name.clone(), symbol.original_name.clone()]
    } else {
        vec![symbol.original_name.clone()]
    }
}

fn expr_from_path(path: Vec<String>, span: Span) -> ast::Expr {
    let mut iter = path.into_iter();
    let Some(first) = iter.next() else {
        return ast::Expr::Unit { span };
    };
    let mut expr = ast::Expr::Identifier { name: first, span };
    for name in iter {
        expr = ast::Expr::Member {
            receiver: Box::new(expr),
            name,
            span,
        };
    }
    expr
}

fn expr_path_ast(expr: &ast::Expr) -> Option<Vec<String>> {
    match expr {
        ast::Expr::Identifier { name, .. } => Some(vec![name.clone()]),
        ast::Expr::Member { receiver, name, .. } => {
            let mut path = expr_path_ast(receiver)?;
            path.push(name.clone());
            Some(path)
        }
        ast::Expr::Group { inner, .. } => expr_path_ast(inner),
        _ => None,
    }
}

#[derive(Clone)]
pub(crate) enum Value {
    Unit,
    Bool(bool),
    Int(i64),
    Float(f64),
    String(String),
    Rune(char),
    Tuple(Vec<Value>),
    List(Rc<RefCell<Vec<Value>>>),
    Set(Rc<RefCell<Vec<Value>>>),
    Map(Rc<RefCell<Vec<(Value, Value)>>>),
    Record(Rc<RefCell<Vec<(String, Value)>>>),
    Aggregate(Rc<RefCell<AggregateValue>>),
    Iterator(Rc<RefCell<IteratorState>>),
    Closure(Rc<ClosureValue>),
    RuntimeType(RuntimeTypeValue),
    RuntimeField {
        owner: runtime::RuntimeTypeId,
        case_id: Option<runtime::RuntimeEnumCaseId>,
        slot: runtime::RuntimeFieldSlot,
    },
    RuntimeMethod {
        owner: runtime::RuntimeTypeId,
        slot: runtime::RuntimeMethodSlot,
    },
    RuntimeParam {
        owner: runtime::RuntimeTypeId,
        method_slot: runtime::RuntimeMethodSlot,
        index: usize,
    },
    RuntimeEnumCase {
        owner: runtime::RuntimeTypeId,
        case_id: runtime::RuntimeEnumCaseId,
    },
}

#[derive(Clone)]
pub(crate) enum RuntimeTypeValue {
    Runtime(runtime::RuntimeTypeId),
    Primitive(String),
    Tuple(Vec<ir::Type>),
    Function {
        params: Vec<ir::Type>,
        ret: Box<ir::Type>,
    },
    AnonymousShape(Vec<ir::NamedType>),
    Unknown,
}

impl Value {
    pub(crate) fn list(items: Vec<Value>) -> Self {
        Self::List(Rc::new(RefCell::new(items)))
    }

    pub(crate) fn set(items: Vec<Value>) -> Self {
        Self::Set(Rc::new(RefCell::new(items)))
    }

    pub(crate) fn map(entries: Vec<(Value, Value)>) -> Self {
        Self::Map(Rc::new(RefCell::new(entries)))
    }

    pub(crate) fn iterator_from_values(items: Vec<Value>) -> Self {
        Self::Iterator(Rc::new(RefCell::new(IteratorState::List {
            items: Rc::new(RefCell::new(items)),
            index: 0,
        })))
    }

    pub(crate) fn variant_case_ids_and_fields(
        &self,
    ) -> Option<(
        runtime::RuntimeTypeId,
        runtime::RuntimeEnumCaseId,
        Vec<Value>,
    )> {
        let Value::Aggregate(aggregate) = self else {
            return None;
        };
        let aggregate = aggregate.borrow();
        Some((
            aggregate.runtime_type_id?,
            aggregate.case_id?,
            aggregate.fields.clone(),
        ))
    }

    pub(crate) fn render(&self) -> String {
        match self {
            Value::Unit => "()".to_string(),
            Value::Bool(value) => value.to_string(),
            Value::Int(value) => value.to_string(),
            Value::Float(value) => {
                let mut rendered = value.to_string();
                if !rendered.contains('.') && !rendered.contains('e') && !rendered.contains('E') {
                    rendered.push_str(".0");
                }
                rendered
            }
            Value::String(value) => value.clone(),
            Value::Rune(value) => value.to_string(),
            Value::Tuple(items) => format!(
                "({})",
                items
                    .iter()
                    .map(Value::render)
                    .collect::<Vec<_>>()
                    .join(",")
            ),
            Value::List(items) => format!(
                "[{}]",
                items
                    .borrow()
                    .iter()
                    .map(Value::render)
                    .collect::<Vec<_>>()
                    .join(",")
            ),
            Value::Set(items) => format!(
                "Set({})",
                items
                    .borrow()
                    .iter()
                    .map(Value::render)
                    .collect::<Vec<_>>()
                    .join(",")
            ),
            Value::Map(entries) => format!(
                "Map({})",
                entries
                    .borrow()
                    .iter()
                    .map(|(key, value)| format!("{}: {}", key.render(), value.render()))
                    .collect::<Vec<_>>()
                    .join(",")
            ),
            Value::Record(fields) => {
                let fields = fields.borrow();
                format!(
                    "shape{{{}}}",
                    fields
                        .iter()
                        .map(|(name, value)| format!("{name}={}", value.render()))
                        .collect::<Vec<_>>()
                        .join(",")
                )
            }
            Value::Aggregate(aggregate) => {
                let aggregate = aggregate.borrow();
                if let Some(case_name) = &aggregate.case_name {
                    if aggregate.fields.is_empty() {
                        case_name.clone()
                    } else {
                        format!(
                            "{}({})",
                            case_name,
                            aggregate
                                .fields
                                .iter()
                                .map(Value::render)
                                .collect::<Vec<_>>()
                                .join(",")
                        )
                    }
                } else {
                    let fields = aggregate
                        .field_names
                        .iter()
                        .zip(aggregate.fields.iter())
                        .map(|(name, value)| format!("{name}={}", value.render()))
                        .collect::<Vec<_>>()
                        .join(",");
                    format!("{}{{{fields}}}", aggregate.type_name)
                }
            }
            Value::Iterator(_) => "<iterator>".to_string(),
            Value::Closure(_) => "<closure>".to_string(),
            Value::RuntimeType(runtime_type) => format!("type {}", runtime_type.render()),
            Value::RuntimeField { .. } => "<field>".to_string(),
            Value::RuntimeMethod { .. } => "<method>".to_string(),
            Value::RuntimeParam { .. } => "<param>".to_string(),
            Value::RuntimeEnumCase { .. } => "<enum-case>".to_string(),
        }
    }
}

impl RuntimeTypeValue {
    fn render(&self) -> String {
        match self {
            RuntimeTypeValue::Runtime(_) => "<runtime>".to_string(),
            RuntimeTypeValue::Primitive(name) => name.clone(),
            RuntimeTypeValue::Tuple(items) => format!(
                "({})",
                items
                    .iter()
                    .map(render_ir_type)
                    .collect::<Vec<_>>()
                    .join(",")
            ),
            RuntimeTypeValue::Function { params, ret } => format!(
                "({}) -> {}",
                params
                    .iter()
                    .map(render_ir_type)
                    .collect::<Vec<_>>()
                    .join(","),
                render_ir_type(ret)
            ),
            RuntimeTypeValue::AnonymousShape(fields) => format!(
                "{{{}}}",
                fields
                    .iter()
                    .map(|field| format!("{} {}", field.name, render_ir_type(&field.ty)))
                    .collect::<Vec<_>>()
                    .join(",")
            ),
            RuntimeTypeValue::Unknown => "<unknown>".to_string(),
        }
    }
}

impl fmt::Debug for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.render())
    }
}

#[derive(Debug, Clone)]
pub(crate) struct AggregateValue {
    runtime_type_id: Option<runtime::RuntimeTypeId>,
    type_name: String,
    kind: crate::ast::TypeKind,
    case_id: Option<runtime::RuntimeEnumCaseId>,
    case_name: Option<String>,
    field_names: Vec<String>,
    fields: Vec<Value>,
}

#[derive(Debug, Clone)]
pub(crate) struct ClosureValue {
    function: ir::FunctionId,
    captures: Vec<Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RuntimeLiftFamily {
    Option,
    Result,
    Either,
}

enum RuntimeLiftMember {
    Success {
        family: RuntimeLiftFamily,
        value: Value,
    },
    Failure {
        family: RuntimeLiftFamily,
        value: Option<Value>,
    },
}

#[derive(Debug, Clone)]
pub(crate) enum IteratorState {
    List {
        items: Rc<RefCell<Vec<Value>>>,
        index: usize,
    },
    Range {
        current: i64,
        end: i64,
        step: i64,
    },
}

#[derive(Debug, Clone)]
struct Frame {
    function: ir::FunctionId,
    locals: Vec<Value>,
    defers: Vec<Rc<ClosureValue>>,
}

pub(crate) struct Interpreter<'a> {
    program: &'a ir::Program,
    runtime: runtime::RuntimeProgram,
    globals: Vec<Value>,
    globals_ready: bool,
    singletons: Vec<Option<Value>>,
    output: String,
}

impl<'a> Interpreter<'a> {
    fn new(program: &'a ir::Program) -> Self {
        let runtime = runtime::RuntimeProgram::from_ir(program);
        let singleton_count = runtime.types.len();
        let mut interpreter = Self {
            program,
            runtime,
            globals: Vec::new(),
            globals_ready: false,
            singletons: vec![None; singleton_count],
            output: String::new(),
        };
        interpreter.globals = interpreter
            .program
            .globals
            .iter()
            .map(|global| interpreter.default_value_for_type(&global.ty))
            .collect();
        interpreter
    }

    fn run(&mut self, requested_entry: Option<&str>) -> Result<Option<Value>, Diagnostic> {
        self.ensure_globals()?;
        let entry = self.select_entry(requested_entry)?;
        let value = self.call_function(entry, None, None, Vec::new(), None)?;
        Ok((!matches!(value, Value::Unit)).then_some(value))
    }

    fn select_entry(&self, requested_entry: Option<&str>) -> Result<ir::FunctionId, Diagnostic> {
        if let Some(name) = requested_entry {
            return self
                .program
                .functions
                .iter()
                .find(|function| function.name == name)
                .map(|function| function.id)
                .ok_or_else(|| self.runtime_error(None, format!("unknown entry '{name}'")));
        }
        if let Some(entry) = self.program.entry {
            return Ok(entry);
        }
        self.program
            .functions
            .iter()
            .find(|function| function.name == "main")
            .or_else(|| {
                self.program
                    .functions
                    .iter()
                    .find(|function| function.name == "run")
            })
            .map(|function| function.id)
            .ok_or_else(|| {
                self.runtime_error(
                    None,
                    "no entry function found; expected lowered 'main' or a top-level 'run'",
                )
            })
    }

    fn ensure_globals(&mut self) -> Result<(), Diagnostic> {
        if self.globals_ready {
            return Ok(());
        }
        if let Some(init) = self.program.global_init {
            let _ = self.call_function(init, None, None, Vec::new(), None)?;
            self.globals_ready = true;
            return Ok(());
        }
        for global in &self.program.globals {
            if let Some(initializer) = &global.initializer {
                let value = self.eval_rvalue(initializer, None, None)?;
                self.globals[global.id.0] = value;
            }
        }
        self.globals_ready = true;
        Ok(())
    }

    fn call_function(
        &mut self,
        id: ir::FunctionId,
        receiver: Option<Value>,
        captures: Option<Vec<Value>>,
        args: Vec<Value>,
        span: Option<Span>,
    ) -> Result<Value, Diagnostic> {
        let function = self
            .program
            .function(id)
            .cloned()
            .ok_or_else(|| self.runtime_error(span, format!("unknown function id {}", id.0)))?;
        let mut frame = Frame {
            function: function.id,
            locals: function
                .locals
                .iter()
                .map(|local| self.default_value_for_type(&local.ty))
                .collect(),
            defers: Vec::new(),
        };

        if matches!(function.kind, ir::FunctionKind::Method { .. }) {
            let Some(receiver) = receiver else {
                return Err(self.runtime_error(
                    span,
                    format!("method '{}' was called without a receiver", function.name),
                ));
            };
            if let Some(first_local) = function.locals.first() {
                frame.locals[first_local.id.0] = receiver;
            }
        }

        let capture_slots = function
            .locals
            .iter()
            .filter(|local| {
                matches!(local.kind, ir::LocalKind::Capture)
                    && !(matches!(function.kind, ir::FunctionKind::Method { .. })
                        && local.name == "this")
            })
            .map(|local| local.id)
            .collect::<Vec<_>>();
        let captures = captures.unwrap_or_default();
        if captures.len() != capture_slots.len() {
            return Err(self.runtime_error(
                span,
                format!(
                    "function '{}' expects {} captures, got {}",
                    function.name,
                    capture_slots.len(),
                    captures.len()
                ),
            ));
        }
        for (slot, value) in capture_slots.into_iter().zip(captures) {
            frame.locals[slot.0] = value;
        }

        let args = self.normalize_call_args(&function, args, span)?;

        if args.len() != function.params.len() {
            return Err(self.runtime_error(
                span,
                format!(
                    "function '{}' expects {} arguments, got {}",
                    function.name,
                    function.params.len(),
                    args.len()
                ),
            ));
        }

        for (param, value) in function.params.iter().zip(args) {
            let ty = function
                .locals
                .get(param.0)
                .map(|local| local.ty.clone())
                .unwrap_or(ir::Type::Unknown);
            let coerced = self.coerce_value_to_type(value, &ty);
            frame.locals[param.0] = coerced;
        }

        let mut block_id = function.entry;
        loop {
            let block = function.block(block_id).cloned().ok_or_else(|| {
                self.runtime_error(span, format!("unknown block id {}", block_id.0))
            })?;
            for statement in block.statements {
                self.exec_statement(&mut frame, statement)?;
            }

            match block.terminator.kind {
                ir::TerminatorKind::Goto(target) => block_id = target,
                ir::TerminatorKind::Branch {
                    condition,
                    then_block,
                    else_block,
                } => {
                    if self
                        .eval_operand(&frame, &condition, block.terminator.span)?
                        .as_bool(self, block.terminator.span, "branch condition")?
                    {
                        block_id = then_block;
                    } else {
                        block_id = else_block;
                    }
                }
                ir::TerminatorKind::Switch {
                    scrutinee,
                    arms,
                    default,
                } => {
                    let scrutinee = self.eval_operand(&frame, &scrutinee, block.terminator.span)?;
                    let mut matched = None;
                    for arm in arms {
                        if self.switch_matches(&scrutinee, &arm.value) {
                            matched = Some(arm.target);
                            break;
                        }
                    }
                    block_id = matched.unwrap_or(default);
                }
                ir::TerminatorKind::Return(value) => {
                    let returned = value
                        .map(|operand| self.eval_operand(&frame, &operand, block.terminator.span))
                        .transpose()?
                        .unwrap_or(Value::Unit);
                    while let Some(deferred) = frame.defers.pop() {
                        let _ = self.call_function(
                            deferred.function,
                            None,
                            Some(deferred.captures.clone()),
                            Vec::new(),
                            block.terminator.span,
                        )?;
                    }
                    if function.name == "new" {
                        if let (Some(Value::Aggregate(receiver)), Value::Aggregate(result)) =
                            (frame.locals.first().cloned(), returned.clone())
                        {
                            let result = result.borrow();
                            let mut receiver = receiver.borrow_mut();
                            if receiver.type_name == result.type_name
                                && receiver.case_name == result.case_name
                            {
                                receiver.fields = result.fields.clone();
                                receiver.field_names = result.field_names.clone();
                                return Ok(Value::Unit);
                            }
                        }
                    }
                    return Ok(self.coerce_value_to_type(returned, &function.return_ty));
                }
                ir::TerminatorKind::Unreachable => {
                    return Err(self.runtime_error(
                        block.terminator.span,
                        format!("entered unreachable block in '{}'", function.name),
                    ));
                }
            }
        }
    }

    fn exec_statement(
        &mut self,
        frame: &mut Frame,
        statement: ir::Statement,
    ) -> Result<(), Diagnostic> {
        match statement.kind {
            ir::StatementKind::Assign { target, value } => {
                let value = self.eval_rvalue(&value, Some(frame), statement.span)?;
                self.assign_place(frame, &target, value, statement.span)
            }
            ir::StatementKind::Defer { value } => {
                let value = self.eval_rvalue(&value, Some(frame), statement.span)?;
                let Value::Closure(closure) = value else {
                    return Err(
                        self.runtime_error(statement.span, "defer expects a lowered closure value")
                    );
                };
                frame.defers.push(closure);
                Ok(())
            }
            ir::StatementKind::Eval { value } => {
                let _ = self.eval_rvalue(&value, Some(frame), statement.span)?;
                Ok(())
            }
        }
    }

    fn normalize_call_args(
        &self,
        function: &ir::Function,
        args: Vec<Value>,
        span: Option<Span>,
    ) -> Result<Vec<Value>, Diagnostic> {
        if let Some(variadic_index) = function.param_variadic.iter().position(|value| *value) {
            if args.len() < variadic_index
                && !function.param_defaults[args.len()..variadic_index]
                    .iter()
                    .all(Option::is_some)
            {
                return Err(self.runtime_error(
                    span,
                    format!(
                        "function '{}' expects at least {} arguments, got {}",
                        function.name,
                        variadic_index,
                        args.len()
                    ),
                ));
            }
            let mut normalized = args
                .iter()
                .take(variadic_index)
                .cloned()
                .collect::<Vec<_>>();
            for default in &function.param_defaults[normalized.len()..variadic_index] {
                let value = default
                    .as_ref()
                    .map(|constant| self.constant_value(constant))
                    .unwrap_or(Value::Unit);
                normalized.push(value);
            }
            let Some(variadic_local) = function
                .params
                .get(variadic_index)
                .and_then(|param| function.locals.get(param.0))
            else {
                return Ok(normalized);
            };
            if args.len() == variadic_index {
                let value = function.param_defaults[variadic_index]
                    .as_ref()
                    .map(|constant| self.constant_value(constant))
                    .unwrap_or_else(|| Value::list(Vec::new()));
                normalized.push(value);
                return Ok(normalized);
            }
            if args.len() == function.params.len()
                && self.value_matches_type(&args[variadic_index], &variadic_local.ty)
            {
                normalized.push(args[variadic_index].clone());
                return Ok(normalized);
            }
            normalized.push(Value::list(args.into_iter().skip(variadic_index).collect()));
            return Ok(normalized);
        }
        if args.len() == function.params.len() {
            return Ok(args);
        }
        if args.len() == 1 {
            if let Value::Tuple(items) = &args[0] {
                if items.len() == function.params.len() {
                    return Ok(items.clone());
                }
            }
        }
        if function.params.len() == 1 && args.len() > 1 {
            return Ok(vec![Value::Tuple(args)]);
        }
        if args.len() < function.params.len()
            && function.param_defaults[args.len()..]
                .iter()
                .all(Option::is_some)
        {
            let mut normalized = args;
            for default in &function.param_defaults[normalized.len()..] {
                let value = default
                    .as_ref()
                    .map(|constant| self.constant_value(constant))
                    .unwrap_or(Value::Unit);
                normalized.push(value);
            }
            return Ok(normalized);
        }
        Err(self.runtime_error(
            span,
            format!(
                "function '{}' expects {} arguments, got {}",
                function.name,
                function.params.len(),
                args.len()
            ),
        ))
    }

    fn variadic_element_type<'t>(
        &'t self,
        function: &'t ir::Function,
        index: usize,
    ) -> Option<&'t ir::Type> {
        let local = function.locals.get(function.params.get(index)?.0)?;
        match &local.ty {
            ir::Type::Named { name, args } if name == "List" && args.len() == 1 => args.first(),
            _ => None,
        }
    }

    fn eval_rvalue(
        &mut self,
        value: &ir::RValue,
        frame: Option<&Frame>,
        span: Option<Span>,
    ) -> Result<Value, Diagnostic> {
        match value {
            ir::RValue::Use(operand) => self.eval_operand_ref(frame, operand, span),
            ir::RValue::Unary { op, operand } => {
                let operand = self.eval_operand_ref(frame, operand, span)?;
                self.eval_unary(*op, operand, span)
            }
            ir::RValue::Binary { op, left, right } => {
                if matches!(op, ir::BinaryOp::And | ir::BinaryOp::Or) {
                    let left = self.eval_operand_ref(frame, left, span)?;
                    let left_bool = left.as_bool(self, span, "left side of boolean operator")?;
                    return match op {
                        ir::BinaryOp::And => {
                            if !left_bool {
                                Ok(Value::Bool(false))
                            } else {
                                let right = self.eval_operand_ref(frame, right, span)?;
                                Ok(Value::Bool(right.as_bool(
                                    self,
                                    span,
                                    "right side of &&",
                                )?))
                            }
                        }
                        ir::BinaryOp::Or => {
                            if left_bool {
                                Ok(Value::Bool(true))
                            } else {
                                let right = self.eval_operand_ref(frame, right, span)?;
                                Ok(Value::Bool(right.as_bool(
                                    self,
                                    span,
                                    "right side of ||",
                                )?))
                            }
                        }
                        _ => unreachable!(),
                    };
                }
                let left = self.eval_operand_ref(frame, left, span)?;
                let right = self.eval_operand_ref(frame, right, span)?;
                self.eval_binary(*op, left, right, span)
            }
            ir::RValue::Call {
                callee,
                args,
                structural,
            } => {
                let args = args
                    .iter()
                    .map(|arg| self.eval_operand_ref(frame, arg, span))
                    .collect::<Result<Vec<_>, _>>()?;
                self.invoke_callee(frame, callee, args, span, *structural)
            }
            ir::RValue::NamedValue { path } => self
                .resolve_named_value_path(frame, path, span)?
                .ok_or_else(|| {
                    self.runtime_error(span, format!("unknown value path '{}'", path.join(".")))
                }),
            ir::RValue::Tuple(items) => Ok(Value::Tuple(
                items
                    .iter()
                    .map(|item| self.eval_operand_ref(frame, item, span))
                    .collect::<Result<Vec<_>, _>>()?,
            )),
            ir::RValue::List(items) => Ok(Value::List(Rc::new(RefCell::new(
                items
                    .iter()
                    .map(|item| self.eval_operand_ref(frame, item, span))
                    .collect::<Result<Vec<_>, _>>()?,
            )))),
            ir::RValue::Record(fields) => Ok(Value::Record(Rc::new(RefCell::new(
                fields
                    .iter()
                    .map(|field| {
                        Ok((
                            field.name.clone(),
                            self.eval_operand_ref(frame, &field.value, span)?,
                        ))
                    })
                    .collect::<Result<Vec<_>, Diagnostic>>()?,
            )))),
            ir::RValue::RecordSpread(parts) => self.record_spread_value(frame, parts, span),
            ir::RValue::Lift { value } => {
                let value = self.eval_operand_ref(frame, value, span)?;
                self.lift_value(value, span)
            }
            ir::RValue::AnonymousInterface { methods, .. } => {
                Ok(Value::Record(Rc::new(RefCell::new(
                    methods
                        .iter()
                        .map(|method| {
                            Ok((
                                method.name.clone(),
                                Value::Closure(Rc::new(ClosureValue {
                                    function: method.function,
                                    captures: method
                                        .captures
                                        .iter()
                                        .map(|capture| self.eval_operand_ref(frame, capture, span))
                                        .collect::<Result<Vec<_>, _>>()?,
                                })),
                            ))
                        })
                        .collect::<Result<Vec<_>, Diagnostic>>()?,
                ))))
            }
            ir::RValue::RecordUpdate { base, patch } => {
                let base = self.eval_operand_ref(frame, base, span)?;
                let patch = self.eval_operand_ref(frame, patch, span)?;
                let updates = self.record_spread_runtime_fields(patch, span)?;
                self.record_update_value(base, updates, span)
            }
            ir::RValue::Construct { ty, fields } => self.construct_value(frame, ty, fields, span),
            ir::RValue::Variant {
                enum_name,
                case_name,
                fields,
            } => self.construct_variant_from_named(frame, enum_name, case_name, fields, span),
            ir::RValue::Field { base, name } => {
                let base = self.eval_operand_ref(frame, base, span)?;
                self.get_member(base, name, span)
            }
            ir::RValue::Index { base, index } => {
                let base = self.eval_operand_ref(frame, base, span)?;
                let index = self.eval_operand_ref(frame, index, span)?;
                self.index_value(base, index, span)
            }
            ir::RValue::Cast { operand, .. } => self.eval_operand_ref(frame, operand, span),
            ir::RValue::TypeTest { operand, ty } => {
                let operand = self.eval_operand_ref(frame, operand, span)?;
                Ok(Value::Bool(self.value_matches_type(&operand, ty)))
            }
            ir::RValue::TypeOf { ty } => {
                Ok(Value::RuntimeType(self.runtime_type_value_for_ir_type(ty)))
            }
            ir::RValue::Closure { function, captures } => {
                Ok(Value::Closure(Rc::new(ClosureValue {
                    function: *function,
                    captures: captures
                        .iter()
                        .map(|capture| self.eval_operand_ref(frame, capture, span))
                        .collect::<Result<Vec<_>, _>>()?,
                })))
            }
        }
    }

    fn eval_operand(
        &mut self,
        frame: &Frame,
        operand: &ir::Operand,
        span: Option<Span>,
    ) -> Result<Value, Diagnostic> {
        self.eval_operand_ref(Some(frame), operand, span)
    }

    fn eval_operand_ref(
        &mut self,
        frame: Option<&Frame>,
        operand: &ir::Operand,
        span: Option<Span>,
    ) -> Result<Value, Diagnostic> {
        match operand {
            ir::Operand::Copy(place) | ir::Operand::Move(place) => {
                self.read_place(frame, place, span)
            }
            ir::Operand::Const(constant) => Ok(self.constant_value(constant)),
        }
    }

    fn constant_value(&self, constant: &ir::Constant) -> Value {
        match constant {
            ir::Constant::Unit => Value::Unit,
            ir::Constant::Bool(value) => Value::Bool(*value),
            ir::Constant::Int(value) => Value::Int(*value),
            ir::Constant::Float(value) => Value::Float(*value),
            ir::Constant::String(value) => Value::String(decode_string_literal(value)),
            ir::Constant::List(items) => {
                Value::list(items.iter().map(|item| self.constant_value(item)).collect())
            }
        }
    }

    fn default_value_for_type(&self, ty: &ir::Type) -> Value {
        match ty {
            ir::Type::Unit => Value::Unit,
            ir::Type::Bool => Value::Bool(false),
            ir::Type::Int => Value::Int(0),
            ir::Type::Float => Value::Float(0.0),
            ir::Type::Str => Value::String(String::new()),
            ir::Type::Tuple(items) => Value::Tuple(
                items
                    .iter()
                    .map(|item| self.default_value_for_type(item))
                    .collect(),
            ),
            ir::Type::Record(fields) => Value::Record(Rc::new(RefCell::new(
                fields
                    .iter()
                    .map(|field| (field.name.clone(), self.default_value_for_type(&field.ty)))
                    .collect(),
            ))),
            ir::Type::Named { name, args } if name == "List" || name == "Array" => {
                let _ = args;
                Value::list(Vec::new())
            }
            ir::Type::Named { name, args } if name == "Set" => {
                let _ = args;
                Value::set(Vec::new())
            }
            ir::Type::Named { name, args } if name == "Map" => {
                let _ = args;
                Value::map(Vec::new())
            }
            ir::Type::Named { name, args } if name == "Option" => {
                let _ = args;
                self.option_none()
            }
            ir::Type::Named { name, args } if name == "Result" => {
                let _ = args;
                self.result_err(Value::Unit)
            }
            ir::Type::Named { name, args } if name == "Either" => {
                let _ = args;
                self.either_left(Value::Unit)
            }
            ir::Type::Named { name, args } if name == "Rune" => {
                let _ = args;
                Value::Rune('\0')
            }
            _ => Value::Unit,
        }
    }

    fn runtime_field_default_value(&self, field: &runtime::RuntimeField) -> Value {
        field
            .initializer
            .as_ref()
            .map(|constant| self.constant_value(constant))
            .unwrap_or_else(|| self.default_value_for_type(&field.ty))
    }

    fn allocate_runtime_fields(&self, fields: &[runtime::RuntimeField]) -> Vec<Value> {
        fields
            .iter()
            .map(|field| self.default_value_for_type(&field.ty))
            .collect()
    }

    fn builtin_enum_variant(
        &self,
        enum_name: &str,
        case_name: &str,
        field_names: &[&str],
        fields: Vec<Value>,
    ) -> Value {
        let runtime_type_id = self
            .runtime
            .type_id_by_name_kind(enum_name, crate::ast::TypeKind::Enum);
        let case_id = runtime_type_id
            .and_then(|type_id| self.runtime.enum_case_by_name(type_id, case_name))
            .map(|case| case.id);
        Value::Aggregate(Rc::new(RefCell::new(AggregateValue {
            runtime_type_id,
            type_name: enum_name.to_string(),
            kind: crate::ast::TypeKind::Enum,
            case_id,
            case_name: Some(case_name.to_string()),
            field_names: field_names.iter().map(|name| (*name).to_string()).collect(),
            fields,
        })))
    }

    pub(crate) fn option_none(&self) -> Value {
        self.builtin_enum_variant("Option", "None", &[], Vec::new())
    }

    pub(crate) fn option_some(&self, value: Value) -> Value {
        self.builtin_enum_variant("Option", "Some", &["value"], vec![value])
    }

    pub(crate) fn result_ok(&self, value: Value) -> Value {
        self.builtin_enum_variant("Result", "Ok", &["value"], vec![value])
    }

    pub(crate) fn result_err(&self, error: Value) -> Value {
        self.builtin_enum_variant("Result", "Err", &["error"], vec![error])
    }

    pub(crate) fn either_left(&self, value: Value) -> Value {
        self.builtin_enum_variant("Either", "Left", &["value"], vec![value])
    }

    pub(crate) fn either_right(&self, value: Value) -> Value {
        self.builtin_enum_variant("Either", "Right", &["value"], vec![value])
    }

    fn runtime_type_value_for_ir_type(&self, ty: &ir::Type) -> RuntimeTypeValue {
        match ty {
            ir::Type::Unknown => RuntimeTypeValue::Unknown,
            ir::Type::Never => RuntimeTypeValue::Primitive("Never".to_string()),
            ir::Type::Unit => RuntimeTypeValue::Primitive("Unit".to_string()),
            ir::Type::Bool => RuntimeTypeValue::Primitive("Bool".to_string()),
            ir::Type::Int => RuntimeTypeValue::Primitive("Int".to_string()),
            ir::Type::Float => RuntimeTypeValue::Primitive("Float".to_string()),
            ir::Type::Str => self
                .runtime
                .type_id_by_name_kind("Str", crate::ast::TypeKind::Class)
                .map(RuntimeTypeValue::Runtime)
                .unwrap_or_else(|| RuntimeTypeValue::Primitive("Str".to_string())),
            ir::Type::Named { name, .. } if is_primitive_type_name(name) => {
                RuntimeTypeValue::Primitive(name.clone())
            }
            ir::Type::Named { name, .. } => self
                .runtime
                .type_id_by_name_any_kind(name)
                .map(RuntimeTypeValue::Runtime)
                .unwrap_or_else(|| RuntimeTypeValue::Primitive(name.clone())),
            ir::Type::Tuple(items) => RuntimeTypeValue::Tuple(items.clone()),
            ir::Type::Record(fields) => RuntimeTypeValue::AnonymousShape(fields.clone()),
            ir::Type::Function { params, ret } => RuntimeTypeValue::Function {
                params: params.clone(),
                ret: ret.clone(),
            },
            ir::Type::TypeParam(name) => RuntimeTypeValue::Primitive(name.clone()),
        }
    }

    fn runtime_type_value_for_value(&self, value: &Value) -> RuntimeTypeValue {
        match value {
            Value::Int(_) => RuntimeTypeValue::Primitive("Int".to_string()),
            Value::Float(_) => RuntimeTypeValue::Primitive("Float".to_string()),
            Value::Bool(_) => RuntimeTypeValue::Primitive("Bool".to_string()),
            Value::Unit => RuntimeTypeValue::Primitive("Unit".to_string()),
            Value::Rune(_) => RuntimeTypeValue::Primitive("Rune".to_string()),
            Value::Tuple(items) => RuntimeTypeValue::Tuple(vec![ir::Type::Unknown; items.len()]),
            Value::Record(_) => RuntimeTypeValue::AnonymousShape(Vec::new()),
            Value::Closure(_) => RuntimeTypeValue::Function {
                params: Vec::new(),
                ret: Box::new(ir::Type::Unknown),
            },
            Value::RuntimeType(_) => self.runtime_type_value_for_ir_type(&ir::Type::Named {
                name: "Type".to_string(),
                args: vec![ir::Type::named("Any")],
            }),
            Value::RuntimeField { .. } => {
                self.runtime_type_value_for_ir_type(&ir::Type::named("Field"))
            }
            Value::RuntimeMethod { .. } => {
                self.runtime_type_value_for_ir_type(&ir::Type::named("Method"))
            }
            Value::RuntimeParam { .. } => {
                self.runtime_type_value_for_ir_type(&ir::Type::named("Param"))
            }
            Value::RuntimeEnumCase { .. } => {
                self.runtime_type_value_for_ir_type(&ir::Type::named("EnumCase"))
            }
            _ => self
                .runtime_type_id_for_value(value)
                .map(RuntimeTypeValue::Runtime)
                .unwrap_or(RuntimeTypeValue::Unknown),
        }
    }

    fn runtime_type_id_for_value(&self, value: &Value) -> Option<runtime::RuntimeTypeId> {
        match value {
            Value::List(_) => self
                .runtime
                .type_id_by_name_kind("List", crate::ast::TypeKind::Class),
            Value::Set(_) => self
                .runtime
                .type_id_by_name_kind("Set", crate::ast::TypeKind::Class),
            Value::Map(_) => self
                .runtime
                .type_id_by_name_kind("Map", crate::ast::TypeKind::Class),
            Value::String(_) => self
                .runtime
                .type_id_by_name_kind("Str", crate::ast::TypeKind::Class),
            Value::Aggregate(aggregate) => {
                let aggregate = aggregate.borrow();
                aggregate.runtime_type_id.or_else(|| {
                    self.runtime
                        .type_id_by_name_kind(&aggregate.type_name, aggregate.kind)
                })
            }
            _ => None,
        }
    }

    fn try_invoke_runtime_method(
        &mut self,
        receiver: Value,
        method: &str,
        args: Vec<Value>,
        span: Option<Span>,
    ) -> Result<Option<Value>, Diagnostic> {
        if let Some(value) =
            self.try_invoke_metadata_method(receiver.clone(), method, args.clone(), span)?
        {
            return Ok(Some(value));
        }

        let Some(type_id) = self.runtime_type_id_for_value(&receiver) else {
            return Ok(None);
        };
        let Some(runtime_ty) = self.runtime.type_by_id(type_id) else {
            return Ok(None);
        };
        if runtime_ty.ir_type_id.is_some() {
            return Ok(None);
        }
        let Some(runtime_method) = self
            .choose_runtime_method_overload(&runtime_ty.methods, method, &args)
            .cloned()
        else {
            return Ok(None);
        };

        let value = match runtime_method.target {
            runtime::RuntimeMethodTarget::Ir(function) => {
                self.call_function(function, Some(receiver), None, args, span)?
            }
            runtime::RuntimeMethodTarget::Builtin(handler) => handler(self, receiver, args, span)?,
        };
        Ok(Some(value))
    }

    fn choose_runtime_method_overload<'m>(
        &self,
        methods: &'m [runtime::RuntimeMethod],
        name: &str,
        args: &[Value],
    ) -> Option<&'m runtime::RuntimeMethod> {
        let mut best = None;
        let mut best_score = i32::MIN;

        for method in methods.iter().filter(|candidate| candidate.name == name) {
            if method.params.len() != args.len() {
                continue;
            }

            let mut score = 10;
            let mut matches = true;
            for (param, arg) in method.params.iter().zip(args) {
                if !self.value_matches_type(arg, param) {
                    matches = false;
                    break;
                }
                if !matches!(param, ir::Type::Unknown | ir::Type::TypeParam(_)) {
                    score += 2;
                }
            }

            if matches && score > best_score {
                best = Some(method);
                best_score = score;
            }
        }

        best
    }

    fn try_invoke_metadata_method(
        &mut self,
        receiver: Value,
        method: &str,
        args: Vec<Value>,
        span: Option<Span>,
    ) -> Result<Option<Value>, Diagnostic> {
        let value = match receiver {
            Value::RuntimeType(runtime_type) => {
                self.invoke_runtime_type_metadata_method(runtime_type, method, args, span)?
            }
            Value::RuntimeField {
                owner,
                case_id,
                slot,
            } => {
                self.invoke_runtime_field_metadata_method(owner, case_id, slot, method, args, span)?
            }
            Value::RuntimeMethod { owner, slot } => {
                self.invoke_runtime_method_metadata_method(owner, slot, method, args, span)?
            }
            Value::RuntimeParam {
                owner,
                method_slot,
                index,
            } => self.invoke_runtime_param_metadata_method(
                owner,
                method_slot,
                index,
                method,
                args,
                span,
            )?,
            Value::RuntimeEnumCase { owner, case_id } => {
                self.invoke_runtime_enum_case_metadata_method(owner, case_id, method, args, span)?
            }
            _ => return Ok(None),
        };
        Ok(Some(value))
    }

    fn invoke_runtime_type_metadata_method(
        &mut self,
        runtime_type: RuntimeTypeValue,
        method: &str,
        args: Vec<Value>,
        span: Option<Span>,
    ) -> Result<Value, Diagnostic> {
        match method {
            "annotation" => {
                self.expect_metadata_arity(method, &args, 1, span)?;
                Ok(self.option_none())
            }
            "hasAnnotation" => {
                self.expect_metadata_arity(method, &args, 1, span)?;
                Ok(Value::Bool(false))
            }
            "name" => {
                self.expect_metadata_arity(method, &args, 0, span)?;
                Ok(self
                    .runtime_type_name(&runtime_type)
                    .map(Value::String)
                    .map(|value| self.option_some(value))
                    .unwrap_or_else(|| self.option_none()))
            }
            "qualifiedName" => {
                self.expect_metadata_arity(method, &args, 0, span)?;
                Ok(self
                    .runtime_type_qualified_name(&runtime_type)
                    .map(Value::String)
                    .map(|value| self.option_some(value))
                    .unwrap_or_else(|| self.option_none()))
            }
            "kind" => {
                self.expect_metadata_arity(method, &args, 0, span)?;
                Ok(self.runtime_type_kind_value(&runtime_type))
            }
            "asClass" => self.runtime_type_cast_value(runtime_type, "Class", method, args, span),
            "asShape" => self.runtime_type_cast_value(runtime_type, "Shape", method, args, span),
            "asEnum" => self.runtime_type_cast_value(runtime_type, "Enum", method, args, span),
            "asInterface" => {
                self.runtime_type_cast_value(runtime_type, "Interface", method, args, span)
            }
            "asSingle" => self.runtime_type_cast_value(runtime_type, "Single", method, args, span),
            "asAnnotation" => {
                self.runtime_type_cast_value(runtime_type, "Annotation", method, args, span)
            }
            "fields" => {
                self.expect_metadata_arity(method, &args, 0, span)?;
                Ok(self.runtime_type_fields_value(&runtime_type))
            }
            "methods" => {
                self.expect_metadata_arity(method, &args, 0, span)?;
                Ok(self.runtime_type_methods_value(&runtime_type))
            }
            "field" => {
                self.expect_metadata_arity(method, &args, 1, span)?;
                let name = self.expect_metadata_string_arg(method, &args[0], span)?;
                Ok(self.runtime_type_field_value(&runtime_type, &name))
            }
            "method" => {
                self.expect_metadata_arity(method, &args, 1, span)?;
                let name = self.expect_metadata_string_arg(method, &args[0], span)?;
                Ok(self.runtime_type_method_value(&runtime_type, &name))
            }
            "cases" => {
                self.expect_metadata_arity(method, &args, 0, span)?;
                Ok(self.runtime_type_enum_cases_value(&runtime_type))
            }
            "case" => {
                self.expect_metadata_arity(method, &args, 1, span)?;
                let name = self.expect_metadata_string_arg(method, &args[0], span)?;
                Ok(self.runtime_type_enum_case_value(&runtime_type, &name))
            }
            _ => Err(self.unknown_metadata_method(method, span)),
        }
    }

    fn invoke_runtime_field_metadata_method(
        &mut self,
        owner: runtime::RuntimeTypeId,
        case_id: Option<runtime::RuntimeEnumCaseId>,
        slot: runtime::RuntimeFieldSlot,
        method: &str,
        args: Vec<Value>,
        span: Option<Span>,
    ) -> Result<Value, Diagnostic> {
        match method {
            "annotation" => {
                self.expect_metadata_arity(method, &args, 1, span)?;
                Ok(self.option_none())
            }
            "hasAnnotation" => {
                self.expect_metadata_arity(method, &args, 1, span)?;
                Ok(Value::Bool(false))
            }
            "name" => {
                self.expect_metadata_arity(method, &args, 0, span)?;
                Ok(Value::String(
                    self.runtime_field_metadata(owner, case_id, slot)
                        .map(|field| field.name.clone())
                        .unwrap_or_default(),
                ))
            }
            "fieldType" => {
                self.expect_metadata_arity(method, &args, 0, span)?;
                let ty = self
                    .runtime_field_metadata(owner, case_id, slot)
                    .map(|field| field.ty.clone())
                    .unwrap_or(ir::Type::Unknown);
                Ok(Value::RuntimeType(self.runtime_type_value_for_ir_type(&ty)))
            }
            "isHidden" => {
                self.expect_metadata_arity(method, &args, 0, span)?;
                Ok(Value::Bool(
                    self.runtime_field_metadata(owner, case_id, slot)
                        .map(|field| field.hidden)
                        .unwrap_or(false),
                ))
            }
            _ => Err(self.unknown_metadata_method(method, span)),
        }
    }

    fn invoke_runtime_method_metadata_method(
        &mut self,
        owner: runtime::RuntimeTypeId,
        slot: runtime::RuntimeMethodSlot,
        method: &str,
        args: Vec<Value>,
        span: Option<Span>,
    ) -> Result<Value, Diagnostic> {
        match method {
            "annotation" => {
                self.expect_metadata_arity(method, &args, 1, span)?;
                Ok(self.option_none())
            }
            "hasAnnotation" => {
                self.expect_metadata_arity(method, &args, 1, span)?;
                Ok(Value::Bool(false))
            }
            "name" => {
                self.expect_metadata_arity(method, &args, 0, span)?;
                Ok(Value::String(
                    self.runtime_method_metadata(owner, slot)
                        .map(|method| method.name.clone())
                        .unwrap_or_default(),
                ))
            }
            "params" => {
                self.expect_metadata_arity(method, &args, 0, span)?;
                let count = self
                    .runtime_method_metadata(owner, slot)
                    .map(|method| method.params.len())
                    .unwrap_or(0);
                Ok(Value::list(
                    (0..count)
                        .map(|index| Value::RuntimeParam {
                            owner,
                            method_slot: slot,
                            index,
                        })
                        .collect(),
                ))
            }
            "returnType" => {
                self.expect_metadata_arity(method, &args, 0, span)?;
                let ty = self
                    .runtime_method_metadata(owner, slot)
                    .map(|method| method.return_ty.clone())
                    .unwrap_or(ir::Type::Unknown);
                Ok(Value::RuntimeType(self.runtime_type_value_for_ir_type(&ty)))
            }
            _ => Err(self.unknown_metadata_method(method, span)),
        }
    }

    fn invoke_runtime_param_metadata_method(
        &mut self,
        owner: runtime::RuntimeTypeId,
        method_slot: runtime::RuntimeMethodSlot,
        index: usize,
        method: &str,
        args: Vec<Value>,
        span: Option<Span>,
    ) -> Result<Value, Diagnostic> {
        match method {
            "name" => {
                self.expect_metadata_arity(method, &args, 0, span)?;
                Ok(Value::String(
                    self.runtime_method_metadata(owner, method_slot)
                        .and_then(|method| method.param_names.get(index))
                        .cloned()
                        .unwrap_or_default(),
                ))
            }
            "paramType" => {
                self.expect_metadata_arity(method, &args, 0, span)?;
                let ty = self
                    .runtime_method_metadata(owner, method_slot)
                    .and_then(|method| method.params.get(index))
                    .cloned()
                    .unwrap_or(ir::Type::Unknown);
                Ok(Value::RuntimeType(self.runtime_type_value_for_ir_type(&ty)))
            }
            _ => Err(self.unknown_metadata_method(method, span)),
        }
    }

    fn invoke_runtime_enum_case_metadata_method(
        &mut self,
        owner: runtime::RuntimeTypeId,
        case_id: runtime::RuntimeEnumCaseId,
        method: &str,
        args: Vec<Value>,
        span: Option<Span>,
    ) -> Result<Value, Diagnostic> {
        match method {
            "annotation" => {
                self.expect_metadata_arity(method, &args, 1, span)?;
                Ok(self.option_none())
            }
            "hasAnnotation" => {
                self.expect_metadata_arity(method, &args, 1, span)?;
                Ok(Value::Bool(false))
            }
            "name" => {
                self.expect_metadata_arity(method, &args, 0, span)?;
                Ok(Value::String(
                    self.runtime_enum_case_metadata(owner, case_id)
                        .map(|case| case.name.clone())
                        .unwrap_or_default(),
                ))
            }
            "fields" => {
                self.expect_metadata_arity(method, &args, 0, span)?;
                Ok(Value::list(
                    self.runtime_enum_case_metadata(owner, case_id)
                        .map(|case| {
                            case.fields
                                .iter()
                                .map(|field| Value::RuntimeField {
                                    owner,
                                    case_id: Some(case_id),
                                    slot: field.slot,
                                })
                                .collect()
                        })
                        .unwrap_or_default(),
                ))
            }
            _ => Err(self.unknown_metadata_method(method, span)),
        }
    }

    fn runtime_type_cast_value(
        &self,
        runtime_type: RuntimeTypeValue,
        expected_kind: &str,
        method: &str,
        args: Vec<Value>,
        span: Option<Span>,
    ) -> Result<Value, Diagnostic> {
        self.expect_metadata_arity(method, &args, 0, span)?;
        if self.runtime_type_kind_case(&runtime_type) == expected_kind {
            Ok(self.option_some(Value::RuntimeType(runtime_type)))
        } else {
            Ok(self.option_none())
        }
    }

    fn runtime_type_name(&self, runtime_type: &RuntimeTypeValue) -> Option<String> {
        match runtime_type {
            RuntimeTypeValue::Runtime(type_id) => {
                self.runtime.type_by_id(*type_id).map(|ty| ty.name.clone())
            }
            RuntimeTypeValue::Primitive(name) => Some(name.clone()),
            RuntimeTypeValue::Unknown => Some("Unknown".to_string()),
            RuntimeTypeValue::Tuple(_)
            | RuntimeTypeValue::Function { .. }
            | RuntimeTypeValue::AnonymousShape(_) => None,
        }
    }

    fn runtime_type_qualified_name(&self, runtime_type: &RuntimeTypeValue) -> Option<String> {
        self.runtime_type_name(runtime_type)
    }

    fn runtime_type_kind_case(&self, runtime_type: &RuntimeTypeValue) -> &'static str {
        match runtime_type {
            RuntimeTypeValue::Runtime(type_id) => self
                .runtime
                .type_by_id(*type_id)
                .map(|ty| match ty.kind {
                    crate::ast::TypeKind::Annotation => "Annotation",
                    crate::ast::TypeKind::Class => "Class",
                    crate::ast::TypeKind::Record => "Shape",
                    crate::ast::TypeKind::Single => "Single",
                    crate::ast::TypeKind::Interface => "Interface",
                    crate::ast::TypeKind::Enum => "Enum",
                })
                .unwrap_or("Primitive"),
            RuntimeTypeValue::Primitive(_) | RuntimeTypeValue::Unknown => "Primitive",
            RuntimeTypeValue::Tuple(_) => "Tuple",
            RuntimeTypeValue::Function { .. } => "Function",
            RuntimeTypeValue::AnonymousShape(_) => "AnonymousShape",
        }
    }

    fn runtime_type_kind_value(&self, runtime_type: &RuntimeTypeValue) -> Value {
        self.builtin_enum_variant(
            "TypeKind",
            self.runtime_type_kind_case(runtime_type),
            &[],
            Vec::new(),
        )
    }

    fn runtime_type_fields_value(&self, runtime_type: &RuntimeTypeValue) -> Value {
        let RuntimeTypeValue::Runtime(type_id) = runtime_type else {
            return Value::list(Vec::new());
        };
        let fields = self
            .runtime
            .type_by_id(*type_id)
            .map(|ty| {
                ty.fields
                    .iter()
                    .map(|field| Value::RuntimeField {
                        owner: *type_id,
                        case_id: None,
                        slot: field.slot,
                    })
                    .collect()
            })
            .unwrap_or_default();
        Value::list(fields)
    }

    fn runtime_type_methods_value(&self, runtime_type: &RuntimeTypeValue) -> Value {
        let RuntimeTypeValue::Runtime(type_id) = runtime_type else {
            return Value::list(Vec::new());
        };
        let methods = self
            .runtime
            .type_by_id(*type_id)
            .map(|ty| {
                ty.methods
                    .iter()
                    .map(|method| Value::RuntimeMethod {
                        owner: *type_id,
                        slot: method.slot,
                    })
                    .collect()
            })
            .unwrap_or_default();
        Value::list(methods)
    }

    fn runtime_type_field_value(&self, runtime_type: &RuntimeTypeValue, name: &str) -> Value {
        let RuntimeTypeValue::Runtime(type_id) = runtime_type else {
            return self.option_none();
        };
        self.runtime
            .type_by_id(*type_id)
            .and_then(|ty| ty.fields.iter().find(|field| field.name == name))
            .map(|field| {
                self.option_some(Value::RuntimeField {
                    owner: *type_id,
                    case_id: None,
                    slot: field.slot,
                })
            })
            .unwrap_or_else(|| self.option_none())
    }

    fn runtime_type_method_value(&self, runtime_type: &RuntimeTypeValue, name: &str) -> Value {
        let RuntimeTypeValue::Runtime(type_id) = runtime_type else {
            return self.option_none();
        };
        self.runtime
            .type_by_id(*type_id)
            .and_then(|ty| ty.methods.iter().find(|method| method.name == name))
            .map(|method| {
                self.option_some(Value::RuntimeMethod {
                    owner: *type_id,
                    slot: method.slot,
                })
            })
            .unwrap_or_else(|| self.option_none())
    }

    fn runtime_type_enum_cases_value(&self, runtime_type: &RuntimeTypeValue) -> Value {
        let RuntimeTypeValue::Runtime(type_id) = runtime_type else {
            return Value::list(Vec::new());
        };
        let cases = self
            .runtime
            .type_by_id(*type_id)
            .map(|ty| {
                ty.enum_cases
                    .iter()
                    .map(|case| Value::RuntimeEnumCase {
                        owner: *type_id,
                        case_id: case.id,
                    })
                    .collect()
            })
            .unwrap_or_default();
        Value::list(cases)
    }

    fn runtime_type_enum_case_value(&self, runtime_type: &RuntimeTypeValue, name: &str) -> Value {
        let RuntimeTypeValue::Runtime(type_id) = runtime_type else {
            return self.option_none();
        };
        self.runtime
            .type_by_id(*type_id)
            .and_then(|ty| ty.enum_cases.iter().find(|case| case.name == name))
            .map(|case| {
                self.option_some(Value::RuntimeEnumCase {
                    owner: *type_id,
                    case_id: case.id,
                })
            })
            .unwrap_or_else(|| self.option_none())
    }

    fn runtime_field_metadata(
        &self,
        owner: runtime::RuntimeTypeId,
        case_id: Option<runtime::RuntimeEnumCaseId>,
        slot: runtime::RuntimeFieldSlot,
    ) -> Option<&runtime::RuntimeField> {
        match case_id {
            Some(case_id) => self
                .runtime
                .type_by_id(owner)?
                .enum_cases
                .get(case_id.0)?
                .fields
                .get(slot.0),
            None => self.runtime.type_by_id(owner)?.fields.get(slot.0),
        }
    }

    fn runtime_method_metadata(
        &self,
        owner: runtime::RuntimeTypeId,
        slot: runtime::RuntimeMethodSlot,
    ) -> Option<&runtime::RuntimeMethod> {
        self.runtime.type_by_id(owner)?.methods.get(slot.0)
    }

    fn runtime_enum_case_metadata(
        &self,
        owner: runtime::RuntimeTypeId,
        case_id: runtime::RuntimeEnumCaseId,
    ) -> Option<&runtime::RuntimeEnumCase> {
        self.runtime.type_by_id(owner)?.enum_cases.get(case_id.0)
    }

    fn expect_metadata_arity(
        &self,
        method: &str,
        args: &[Value],
        expected: usize,
        span: Option<Span>,
    ) -> Result<(), Diagnostic> {
        if args.len() == expected {
            return Ok(());
        }
        Err(self.runtime_error(
            span,
            format!(
                "metadata method '{}' expects {} arguments, got {}",
                method,
                expected,
                args.len()
            ),
        ))
    }

    fn expect_metadata_string_arg(
        &self,
        method: &str,
        value: &Value,
        span: Option<Span>,
    ) -> Result<String, Diagnostic> {
        match value {
            Value::String(value) => Ok(value.clone()),
            other => Err(self.runtime_error(
                span,
                format!(
                    "metadata method '{}' expects Str argument, got {}",
                    method,
                    other.render()
                ),
            )),
        }
    }

    fn unknown_metadata_method(&self, method: &str, span: Option<Span>) -> Diagnostic {
        self.runtime_error(
            span,
            format!("metadata method '{}' is not available", method),
        )
    }

    fn read_place(
        &mut self,
        frame: Option<&Frame>,
        place: &ir::Place,
        span: Option<Span>,
    ) -> Result<Value, Diagnostic> {
        match place {
            ir::Place::Local(id) => frame
                .and_then(|frame| frame.locals.get(id.0).cloned())
                .ok_or_else(|| self.runtime_error(span, format!("unknown local {}", id.0))),
            ir::Place::Global(id) => self
                .globals
                .get(id.0)
                .cloned()
                .ok_or_else(|| self.runtime_error(span, format!("unknown global {}", id.0))),
            ir::Place::Field { base, name } => {
                let base = self.eval_operand_ref(frame, base, span)?;
                self.get_member(base, name, span)
            }
            ir::Place::Index { base, index } => {
                let base = self.eval_operand_ref(frame, base, span)?;
                let index = self.eval_operand_ref(frame, index, span)?;
                self.index_value(base, index, span)
            }
        }
    }

    fn assign_place(
        &mut self,
        frame: &mut Frame,
        place: &ir::Place,
        value: Value,
        span: Option<Span>,
    ) -> Result<(), Diagnostic> {
        match place {
            ir::Place::Local(id) => {
                let Some(slot) = frame.locals.get_mut(id.0) else {
                    return Err(self.runtime_error(span, format!("unknown local {}", id.0)));
                };
                let ty = self
                    .program
                    .function(frame.function)
                    .and_then(|function| function.locals.get(id.0))
                    .map(|local| local.ty.clone())
                    .unwrap_or(ir::Type::Unknown);
                *slot = self.coerce_value_to_type(value, &ty);
                Ok(())
            }
            ir::Place::Global(id) => {
                let ty = self
                    .program
                    .globals
                    .get(id.0)
                    .map(|global| global.ty.clone())
                    .unwrap_or(ir::Type::Unknown);
                let coerced = self.coerce_value_to_type(value, &ty);
                let Some(slot) = self.globals.get_mut(id.0) else {
                    return Err(self.runtime_error(span, format!("unknown global {}", id.0)));
                };
                *slot = coerced;
                Ok(())
            }
            ir::Place::Field { base, name } => {
                let base = self.eval_operand(frame, base, span)?;
                self.set_member(base, name, value, span)
            }
            ir::Place::Index { base, index } => {
                let base = self.eval_operand(frame, base, span)?;
                let index = self.eval_operand(frame, index, span)?;
                self.set_index(base, index, value, span)
            }
        }
    }

    fn invoke_callee(
        &mut self,
        frame: Option<&Frame>,
        callee: &ir::Callee,
        args: Vec<Value>,
        span: Option<Span>,
        structural: bool,
    ) -> Result<Value, Diagnostic> {
        match callee {
            ir::Callee::Direct(id) => self.call_function(*id, None, None, args, span),
            ir::Callee::Indirect(value) => {
                let callee = self.eval_operand_ref(frame, value, span)?;
                self.invoke_value(callee, args, span)
            }
            ir::Callee::Method { receiver, method } => {
                let receiver = self.eval_operand_ref(frame, receiver, span)?;
                self.invoke_method(receiver, method, args, span)
            }
            ir::Callee::Intrinsic(intrinsic) => self.invoke_intrinsic(intrinsic, args, span),
            ir::Callee::Named { path } => {
                self.invoke_named_path(frame, path, args, span, structural)
            }
        }
    }

    pub(crate) fn invoke_value(
        &mut self,
        callee: Value,
        args: Vec<Value>,
        span: Option<Span>,
    ) -> Result<Value, Diagnostic> {
        match callee {
            Value::Closure(closure) => self.call_function(
                closure.function,
                None,
                Some(closure.captures.clone()),
                args,
                span,
            ),
            Value::Aggregate(aggregate) => {
                let aggregate = aggregate.borrow();
                Err(self.runtime_error(
                    span,
                    format!("value '{}' is not directly callable", aggregate.type_name),
                ))
            }
            _ => Err(self.runtime_error(span, "indirect callable values are not implemented yet")),
        }
    }

    pub(crate) fn value_is_zero_arg_closure(&self, value: &Value) -> bool {
        let Value::Closure(closure) = value else {
            return false;
        };
        self.program
            .function(closure.function)
            .is_some_and(|function| function.params.is_empty())
    }

    fn invoke_named_path(
        &mut self,
        frame: Option<&Frame>,
        path: &[String],
        args: Vec<Value>,
        span: Option<Span>,
        structural: bool,
    ) -> Result<Value, Diagnostic> {
        if path.is_empty() {
            return Err(self.runtime_error(span, "empty callee path"));
        }

        if path.len() == 1 {
            let name = &path[0];
            if let Some(function) = self.lookup_function(name) {
                return self.call_function(function, None, None, args, span);
            }
            return self.invoke_root_named(name, args, span, structural);
        }

        if path[0] == "OS" && path.len() == 2 {
            return self.invoke_os_method(&path[1], args, span);
        }
        if path[0] == "OS" && path.len() == 3 && matches!(path[1].as_str(), "stdout" | "stderr") {
            return self.invoke_os_method(&path[2], args, span);
        }

        if path[0] == "Array" && path.len() == 2 {
            let method = path[1].as_str();
            match method {
                "ofInt" | "ofFloat" | "ofBool" | "ofStr" | "ofRune" => {
                    if args.len() != 1 {
                        return Err(self.runtime_error(
                            span,
                            format!("Array.{method} expects 1 argument, got {}", args.len()),
                        ));
                    }
                    let context = format!("Array.{method} length");
                    let len = args[0].as_int(self, span, &context)?;
                    if len < 0 {
                        return Err(self.runtime_error(
                            span,
                            format!("Array.{method} length must be non-negative"),
                        ));
                    }
                    let default = match method {
                        "ofInt" => Value::Int(0),
                        "ofFloat" => Value::Float(0.0),
                        "ofBool" => Value::Bool(false),
                        "ofStr" => Value::String(String::new()),
                        "ofRune" => Value::Rune('\0'),
                        _ => unreachable!(),
                    };
                    return Ok(Value::list(vec![default; len as usize]));
                }
                "fill" => {
                    if args.len() != 2 {
                        return Err(self.runtime_error(
                            span,
                            format!("Array.fill expects 2 arguments, got {}", args.len()),
                        ));
                    }
                    let len = args[0].as_int(self, span, "Array.fill length")?;
                    if len < 0 {
                        return Err(
                            self.runtime_error(span, "Array.fill length must be non-negative")
                        );
                    }
                    return Ok(Value::List(Rc::new(RefCell::new(vec![
                        args[1].clone();
                        len as usize
                    ]))));
                }
                "generate" => {
                    if args.len() != 2 {
                        return Err(self.runtime_error(
                            span,
                            format!("Array.generate expects 2 arguments, got {}", args.len()),
                        ));
                    }
                    let len = args[0].as_int(self, span, "Array.generate length")?;
                    if len < 0 {
                        return Err(
                            self.runtime_error(span, "Array.generate length must be non-negative")
                        );
                    }
                    let callback = args[1].clone();
                    let mut values = Vec::with_capacity(len as usize);
                    for index in 0..(len as usize) {
                        values.push(self.invoke_value(
                            callback.clone(),
                            vec![Value::Int(index as i64)],
                            span,
                        )?);
                    }
                    return Ok(Value::List(Rc::new(RefCell::new(values))));
                }
                _ => {}
            }
        }

        if path[0] == "List" && path.len() == 2 && path[1] == "from" {
            if args.len() != 1 {
                return Err(self.runtime_error(
                    span,
                    format!("List.from expects 1 argument, got {}", args.len()),
                ));
            }
            let values = iterable_values(args[0].clone(), span, self)?;
            return Ok(Value::list(values));
        }

        if path[0] == "Set" && path.len() == 2 && path[1] == "from" {
            if args.len() != 1 {
                return Err(self.runtime_error(
                    span,
                    format!("Set.from expects 1 argument, got {}", args.len()),
                ));
            }
            let values = iterable_values(args[0].clone(), span, self)?;
            return Ok(Value::set(unique_values(values)));
        }

        if path[0] == "Int" && path.len() == 2 && path[1] == "parse" {
            if args.len() != 1 {
                return Err(self.runtime_error(
                    span,
                    format!("Int.parse expects 1 argument, got {}", args.len()),
                ));
            }
            let Value::String(text) = &args[0] else {
                return Err(self.runtime_error(
                    span,
                    format!("Int.parse expects Str, got {}", args[0].render()),
                ));
            };
            return Ok(match text.parse::<i64>() {
                Ok(parsed) => self.option_some(Value::Int(parsed)),
                Err(_) => self.option_none(),
            });
        }

        if path[0] == "Float" && path.len() == 2 && path[1] == "parse" {
            if args.len() != 1 {
                return Err(self.runtime_error(
                    span,
                    format!("Float.parse expects 1 argument, got {}", args.len()),
                ));
            }
            let Value::String(text) = &args[0] else {
                return Err(self.runtime_error(
                    span,
                    format!("Float.parse expects Str, got {}", args[0].render()),
                ));
            };
            return Ok(match text.parse::<f64>() {
                Ok(parsed) => self.option_some(Value::Float(parsed)),
                Err(_) => self.option_none(),
            });
        }

        if path[0] == "Option" && path.len() == 2 && path[1] == "when" {
            if args.len() != 2 {
                return Err(self.runtime_error(
                    span,
                    format!("Option.when expects 2 arguments, got {}", args.len()),
                ));
            }
            let condition = args[0].as_bool(self, span, "Option.when condition")?;
            return Ok(if condition {
                self.option_some(args[1].clone())
            } else {
                self.option_none()
            });
        }

        if path.len() == 2 {
            if let Some(value) = self.construct_named_path(path, args.clone(), span, true)? {
                return Ok(value);
            }
        }

        if let Some(receiver) = self.resolve_runtime_path(frame, &path[..path.len() - 1], span)? {
            return self.invoke_method(receiver, &path[path.len() - 1], args, span);
        }

        Err(self.runtime_error(
            span,
            format!("unsupported named callee path '{}'", path.join(".")),
        ))
    }

    fn resolve_runtime_path(
        &mut self,
        frame: Option<&Frame>,
        path: &[String],
        span: Option<Span>,
    ) -> Result<Option<Value>, Diagnostic> {
        self.resolve_named_value_path(frame, path, span)
    }

    fn resolve_named_value_path(
        &mut self,
        frame: Option<&Frame>,
        path: &[String],
        span: Option<Span>,
    ) -> Result<Option<Value>, Diagnostic> {
        let Some(first) = path.first() else {
            return Ok(None);
        };

        if path.len() >= 2 {
            if let Some(mut value) = self.construct_named_path_value(&path[0], &path[1], span)? {
                for segment in &path[2..] {
                    value = self.get_member(value, segment, span)?;
                }
                return Ok(Some(value));
            }
        }

        let Some(mut value) = self.resolve_named_root(frame, first, span)? else {
            return Ok(None);
        };
        for segment in &path[1..] {
            value = self.get_member(value, segment, span)?;
        }
        Ok(Some(value))
    }

    fn resolve_named_root(
        &mut self,
        frame: Option<&Frame>,
        name: &str,
        span: Option<Span>,
    ) -> Result<Option<Value>, Diagnostic> {
        if let Some(value) = self.lookup_runtime_value(frame, name) {
            return Ok(Some(value));
        }
        if let Some(value) = self.lookup_singleton(name, span)? {
            return Ok(Some(value));
        }
        if name == "None" {
            return Ok(Some(self.option_none()));
        }
        match self.construct_enum_case(None, name, Vec::new(), span, false) {
            Ok(value) => Ok(Some(value)),
            Err(error) if error.code == "runtime_error" => Ok(None),
            Err(error) => Err(error),
        }
    }

    fn construct_named_path_value(
        &mut self,
        type_name: &str,
        member: &str,
        span: Option<Span>,
    ) -> Result<Option<Value>, Diagnostic> {
        self.construct_named_path(
            &[type_name.to_string(), member.to_string()],
            Vec::new(),
            span,
            false,
        )
    }

    fn invoke_root_named(
        &mut self,
        name: &str,
        args: Vec<Value>,
        span: Option<Span>,
        structural: bool,
    ) -> Result<Value, Diagnostic> {
        if !structural {
            if let Some(value) = self.construct_builtin(name, &args, span)? {
                return Ok(value);
            }
        }
        if args.is_empty() {
            if let Some(value) = self.lookup_singleton(name, span)? {
                return Ok(value);
            }
        }
        if let Some(value) = self.construct_named_type(name, args.clone(), span, structural)? {
            return Ok(value);
        }
        if let Some(value) = self.lookup_runtime_value(None, name) {
            return self.invoke_value(value, args, span);
        }
        Err(self.runtime_error(span, format!("unknown callable '{}'", name)))
    }

    fn construct_named_path(
        &mut self,
        path: &[String],
        args: Vec<Value>,
        span: Option<Span>,
        from_call: bool,
    ) -> Result<Option<Value>, Diagnostic> {
        if path.len() != 2 {
            return Ok(None);
        }
        let type_name = &path[0];
        let member = &path[1];

        if type_name == "OS" && matches!(member.as_str(), "stdout" | "stderr") && args.is_empty() {
            return self.lookup_singleton("OS", span);
        }

        if self
            .lookup_type_by_kind(type_name, crate::ast::TypeKind::Enum)
            .is_some_and(|ty| ty.enum_cases.iter().any(|case| case.name == *member))
        {
            return self
                .construct_enum_case(Some(type_name), member, args, span, from_call)
                .map(Some);
        }
        Ok(None)
    }

    fn construct_builtin(
        &mut self,
        name: &str,
        args: &[Value],
        span: Option<Span>,
    ) -> Result<Option<Value>, Diagnostic> {
        let value = match name {
            "Range" => {
                if !(args.len() == 2 || args.len() == 3) {
                    return Err(self.runtime_error(
                        span,
                        format!("Range expects 2 or 3 arguments, got {}", args.len()),
                    ));
                }
                let start = args[0].as_int(self, span, "Range start")?;
                let end = args[1].as_int(self, span, "Range end")?;
                let step = if args.len() == 3 {
                    args[2].as_int(self, span, "Range step")?
                } else if start <= end {
                    1
                } else {
                    -1
                };
                Some(Value::Iterator(Rc::new(RefCell::new(
                    IteratorState::Range {
                        current: start,
                        end,
                        step,
                    },
                ))))
            }
            "List" | "Array" => Some(Value::List(Rc::new(RefCell::new(args.to_vec())))),
            "Set" => Some(Value::Set(Rc::new(RefCell::new(unique_values(
                args.to_vec(),
            ))))),
            "Map" => {
                let mut entries = Vec::new();
                for arg in args {
                    let Value::Tuple(items) = arg else {
                        return Err(self.runtime_error(span, "Map expects tuple pair arguments"));
                    };
                    if items.len() != 2 {
                        return Err(self.runtime_error(span, "Map expects tuple pair arguments"));
                    }
                    map_put_entry(&mut entries, items[0].clone(), items[1].clone());
                }
                Some(Value::Map(Rc::new(RefCell::new(entries))))
            }
            "Some" => {
                if args.len() != 1 {
                    return Err(self.runtime_error(span, "Some expects 1 argument"));
                }
                Some(self.option_some(args[0].clone()))
            }
            "None" => {
                return Err(self.runtime_error(
                    span,
                    "enum case 'None' does not accept call syntax; use 'None'",
                ));
            }
            "Ok" => {
                if args.len() != 1 {
                    return Err(self.runtime_error(span, "Ok expects 1 argument"));
                }
                Some(self.result_ok(args[0].clone()))
            }
            "Err" => {
                if args.len() != 1 {
                    return Err(self.runtime_error(span, "Err expects 1 argument"));
                }
                Some(self.result_err(args[0].clone()))
            }
            "Left" => {
                if args.len() != 1 {
                    return Err(self.runtime_error(span, "Left expects 1 argument"));
                }
                Some(self.either_left(args[0].clone()))
            }
            "Right" => {
                if args.len() != 1 {
                    return Err(self.runtime_error(span, "Right expects 1 argument"));
                }
                Some(self.either_right(args[0].clone()))
            }
            _ => None,
        };
        Ok(value)
    }

    fn construct_named_type(
        &mut self,
        type_name: &str,
        args: Vec<Value>,
        span: Option<Span>,
        structural: bool,
    ) -> Result<Option<Value>, Diagnostic> {
        let Some(ty) = self
            .runtime
            .types
            .iter()
            .find(|ty| {
                ty.name == type_name
                    && ty.kind != crate::ast::TypeKind::Enum
                    && ty.kind != crate::ast::TypeKind::Single
            })
            .cloned()
        else {
            return Ok(None);
        };

        let instance = Value::Aggregate(Rc::new(RefCell::new(AggregateValue {
            runtime_type_id: Some(ty.id),
            type_name: type_name.to_string(),
            kind: ty.kind,
            case_id: None,
            case_name: None,
            field_names: ty.fields.iter().map(|field| field.name.clone()).collect(),
            fields: self.allocate_runtime_fields(&ty.fields),
        })));

        if let Some(field_init) = ty.field_init {
            let _ =
                self.call_function(field_init, Some(instance.clone()), None, Vec::new(), span)?;
        }

        let has_explicit_constructor = ty.methods.iter().any(|method| method.name == "new");

        if structural {
            if has_explicit_constructor {
                let constructor_args = match args.as_slice() {
                    [Value::Record(values)] => values
                        .borrow()
                        .iter()
                        .map(|(_, value)| value.clone())
                        .collect::<Vec<_>>(),
                    _ => {
                        return Err(self.runtime_error(
                            span,
                            format!(
                                "brace construction for '{}' expects constructor fields",
                                type_name
                            ),
                        ));
                    }
                };
                if let Some(init) =
                    self.find_method_overload_for_kind(type_name, ty.kind, "new", &constructor_args)
                {
                    let receiver = instance.clone();
                    let _ =
                        self.call_function(init, Some(receiver), None, constructor_args, span)?;
                    return Ok(Some(instance));
                }
                return Err(self.runtime_error(
                    span,
                    format!(
                        "no constructor overload for class '{}' matches {} arguments",
                        type_name,
                        constructor_args.len()
                    ),
                ));
            }

            match args.as_slice() {
                [Value::Record(values)] => {
                    let values = values.borrow();
                    self.apply_named_record_constructor(&instance, &ty, &values, span)?;
                    return Ok(Some(instance));
                }
                _ => {
                    return Err(self.runtime_error(
                        span,
                        format!(
                            "brace-based construction for '{}' expects construction fields",
                            type_name
                        ),
                    ));
                }
            }
        }

        if matches!(args.as_slice(), [Value::Record(_)]) {
            return Err(self.runtime_error(
                span,
                format!(
                    "constructor syntax for '{}' does not accept anonymous shape arguments in '(...)'; use construction fields in braces or positional values directly",
                    type_name
                ),
            ));
        }

        if let Some(init) = self.find_method_overload_for_kind(type_name, ty.kind, "new", &args) {
            let receiver = instance.clone();
            let _ = self.call_function(init, Some(receiver), None, args, span)?;
            return Ok(Some(instance));
        }

        if has_explicit_constructor {
            return Err(self.runtime_error(
                span,
                format!(
                    "no constructor overload for class '{}' matches {} arguments",
                    type_name,
                    args.len()
                ),
            ));
        }

        self.apply_positional_record_constructor(&instance, &ty, &args, span)?;
        Ok(Some(instance))
    }

    fn apply_named_record_constructor(
        &mut self,
        instance: &Value,
        ty: &runtime::RuntimeType,
        values: &[(String, Value)],
        span: Option<Span>,
    ) -> Result<(), Diagnostic> {
        if let Some(field) = ty
            .fields
            .iter()
            .find(|field| field.hidden && !field.has_initializer)
        {
            return Err(self.runtime_error(
                span,
                format!(
                    "class '{}' has no implicit field constructor because hidden field '{}' has no initializer; define 'new' to initialize it",
                    ty.name, field.name
                ),
            ));
        }

        let visible_fields = ty
            .fields
            .iter()
            .filter(|field| !field.hidden)
            .collect::<Vec<_>>();
        let required_visible = visible_fields
            .iter()
            .filter(|field| !field.has_initializer)
            .count();
        if values.len() < required_visible || values.len() > visible_fields.len() {
            return Err(self.runtime_error(
                span,
                format!(
                    "class '{}' requires construction fields that match the visible class shape",
                    ty.name
                ),
            ));
        }

        let mut aggregate = match instance {
            Value::Aggregate(value) => value.borrow_mut(),
            _ => unreachable!(),
        };
        for (name, value) in values {
            let Some(field) = visible_fields.iter().find(|field| field.name == *name) else {
                return Err(self.runtime_error(
                    span,
                    format!(
                        "class '{}' requires construction fields that match the visible class shape",
                        ty.name
                    ),
                ));
            };
            aggregate.fields[field.slot.0] = self.coerce_value_to_type(value.clone(), &field.ty);
        }

        for field in visible_fields {
            if !field.has_initializer && !values.iter().any(|(name, _)| *name == field.name) {
                return Err(self.runtime_error(
                    span,
                    format!(
                        "class '{}' requires construction fields that match the visible class shape",
                        ty.name
                    ),
                ));
            }
        }

        Ok(())
    }

    fn apply_positional_record_constructor(
        &mut self,
        instance: &Value,
        ty: &runtime::RuntimeType,
        values: &[Value],
        span: Option<Span>,
    ) -> Result<(), Diagnostic> {
        if let Some(field) = ty
            .fields
            .iter()
            .find(|field| field.hidden && !field.has_initializer)
        {
            return Err(self.runtime_error(
                span,
                format!(
                    "class '{}' has no implicit positional constructor because hidden field '{}' has no initializer; define 'new' to initialize it",
                    ty.name, field.name
                ),
            ));
        }

        if ty.fields.iter().enumerate().any(|(index, field)| {
            field.hidden
                && field.has_initializer
                && ty.fields[index + 1..].iter().any(|later| !later.hidden)
        }) {
            return Err(self.runtime_error(
                span,
                format!(
                    "class '{}' cannot use positional construction because hidden defaulted fields must come after all visible fields",
                    ty.name
                ),
            ));
        }

        let visible_fields = ty
            .fields
            .iter()
            .filter(|field| !field.hidden)
            .collect::<Vec<_>>();
        if values.len() > visible_fields.len()
            || visible_fields[values.len()..]
                .iter()
                .any(|field| !field.has_initializer)
        {
            return Err(self.runtime_error(
                span,
                format!(
                    "class '{}' positional construction must match visible field order and may omit only trailing defaulted fields",
                    ty.name
                ),
            ));
        }

        let mut aggregate = match instance {
            Value::Aggregate(value) => value.borrow_mut(),
            _ => unreachable!(),
        };
        for (value, field) in values.iter().zip(visible_fields.iter()) {
            aggregate.fields[field.slot.0] = self.coerce_value_to_type(value.clone(), &field.ty);
        }

        Ok(())
    }

    fn construct_value(
        &mut self,
        frame: Option<&Frame>,
        ty: &ir::Type,
        fields: &[ir::NamedOperand],
        span: Option<Span>,
    ) -> Result<Value, Diagnostic> {
        match ty {
            ir::Type::Named { name, .. } => {
                let args = fields
                    .iter()
                    .map(|field| self.eval_operand_ref(frame, &field.value, span))
                    .collect::<Result<Vec<_>, _>>()?;
                self.construct_named_type(name, args, span, false)?
                    .ok_or_else(|| {
                        self.runtime_error(span, format!("cannot construct type '{name}'"))
                    })
            }
            ir::Type::Record(field_types) => {
                let mut out = Vec::new();
                for field in field_types {
                    let value = fields
                        .iter()
                        .find(|named| named.name == field.name)
                        .map(|named| self.eval_operand_ref(frame, &named.value, span))
                        .transpose()?
                        .unwrap_or_else(|| self.default_value_for_type(&field.ty));
                    out.push((field.name.clone(), value));
                }
                Ok(Value::Record(Rc::new(RefCell::new(out))))
            }
            _ => Err(self.runtime_error(
                span,
                "construct is only implemented for named and shape types right now",
            )),
        }
    }

    fn construct_variant_from_named(
        &mut self,
        frame: Option<&Frame>,
        enum_name: &str,
        case_name: &str,
        fields: &[ir::NamedOperand],
        span: Option<Span>,
    ) -> Result<Value, Diagnostic> {
        let values = fields
            .iter()
            .map(|field| {
                Ok((
                    field.name.clone(),
                    self.eval_operand_ref(frame, &field.value, span)?,
                ))
            })
            .collect::<Result<Vec<_>, Diagnostic>>()?;
        let runtime_type_id = self
            .runtime
            .type_id_by_name_kind(enum_name, crate::ast::TypeKind::Enum);
        let case_id = runtime_type_id
            .and_then(|type_id| self.runtime.enum_case_by_name(type_id, case_name))
            .map(|case| case.id);
        let (field_names, fields): (Vec<_>, Vec<_>) = values.into_iter().unzip();
        Ok(Value::Aggregate(Rc::new(RefCell::new(AggregateValue {
            runtime_type_id,
            type_name: enum_name.to_string(),
            kind: crate::ast::TypeKind::Enum,
            case_id,
            case_name: Some(case_name.to_string()),
            field_names,
            fields,
        }))))
    }

    fn record_update_value(
        &mut self,
        base: Value,
        updates: Vec<(String, Value)>,
        span: Option<Span>,
    ) -> Result<Value, Diagnostic> {
        match base {
            Value::Record(fields) => {
                let mut next = fields.borrow().clone();
                for (name, value) in updates {
                    if let Some((_, slot)) = next.iter_mut().find(|(field, _)| *field == name) {
                        *slot = value;
                    } else {
                        return Err(
                            self.runtime_error(span, format!("shape has no field '{}'", name))
                        );
                    }
                }
                Ok(Value::Record(Rc::new(RefCell::new(next))))
            }
            Value::Aggregate(instance) if instance.borrow().case_name.is_none() => {
                let instance = instance.borrow();
                let mut next = instance.fields.clone();
                for (name, value) in updates {
                    if let Some(index) =
                        instance.field_names.iter().position(|field| field == &name)
                    {
                        next[index] = value;
                    } else {
                        return Err(self.runtime_error(
                            span,
                            format!("value '{}' has no field '{}'", instance.type_name, name),
                        ));
                    }
                }
                Ok(Value::Aggregate(Rc::new(RefCell::new(AggregateValue {
                    runtime_type_id: instance.runtime_type_id,
                    type_name: instance.type_name.clone(),
                    kind: instance.kind,
                    case_id: instance.case_id,
                    case_name: instance.case_name.clone(),
                    field_names: instance.field_names.clone(),
                    fields: next,
                }))))
            }
            other => {
                Err(self.runtime_error(span, format!("cannot update fields on {}", other.render())))
            }
        }
    }

    fn record_spread_value(
        &mut self,
        frame: Option<&Frame>,
        parts: &[ir::RecordSpreadPart],
        span: Option<Span>,
    ) -> Result<Value, Diagnostic> {
        let mut out = Vec::new();
        for part in parts {
            match part {
                ir::RecordSpreadPart::Spread(operand) => {
                    let value = self.eval_operand_ref(frame, operand, span)?;
                    for (name, value) in self.record_spread_runtime_fields(value, span)? {
                        push_unique_runtime_record_field(&mut out, name, value, span, self)?;
                    }
                }
                ir::RecordSpreadPart::Field(field) => {
                    let value = self.eval_operand_ref(frame, &field.value, span)?;
                    push_unique_runtime_record_field(
                        &mut out,
                        field.name.clone(),
                        value,
                        span,
                        self,
                    )?;
                }
            }
        }
        Ok(Value::Record(Rc::new(RefCell::new(out))))
    }

    fn record_spread_runtime_fields(
        &mut self,
        value: Value,
        span: Option<Span>,
    ) -> Result<Vec<(String, Value)>, Diagnostic> {
        match value {
            Value::Record(fields) => Ok(fields.borrow().clone()),
            Value::Aggregate(instance) if instance.borrow().case_name.is_none() => {
                let instance = instance.borrow();
                if instance.fields.is_empty() {
                    return Err(self.runtime_error(
                        span,
                        format!(
                            "shape spread requires a shape value, got {}",
                            instance.type_name
                        ),
                    ));
                }
                if let Some(runtime_type_id) = instance.runtime_type_id {
                    if let Some(runtime_ty) = self.runtime.type_by_id(runtime_type_id) {
                        return Ok(runtime_ty
                            .fields
                            .iter()
                            .filter(|field| !field.hidden)
                            .filter_map(|field| {
                                instance
                                    .fields
                                    .get(field.slot.0)
                                    .cloned()
                                    .map(|value| (field.name.clone(), value))
                            })
                            .collect());
                    }
                }
                Ok(instance
                    .field_names
                    .iter()
                    .cloned()
                    .zip(instance.fields.iter().cloned())
                    .collect())
            }
            other => Err(self.runtime_error(
                span,
                format!(
                    "shape spread requires a shape value, got {}",
                    other.render()
                ),
            )),
        }
    }

    fn lift_value(&mut self, value: Value, span: Option<Span>) -> Result<Value, Diagnostic> {
        match value {
            Value::Record(fields) => {
                let fields = fields.borrow().clone();
                if fields.is_empty() {
                    return Err(self.runtime_error(span, "lift requires at least one shape field"));
                }
                let mut family = None;
                let mut lifted = Vec::new();
                for (name, value) in fields {
                    match self.unwrap_runtime_lift_member(value, span)? {
                        RuntimeLiftMember::Success {
                            family: next_family,
                            value,
                        } => {
                            self.merge_runtime_lift_family(&mut family, next_family, span)?;
                            lifted.push((name, value));
                        }
                        RuntimeLiftMember::Failure {
                            family: next_family,
                            value,
                        } => {
                            self.merge_runtime_lift_family(&mut family, next_family, span)?;
                            return Ok(self.wrap_runtime_lift_failure(next_family, value));
                        }
                    }
                }
                let family = family.expect("non-empty lift established a family");
                Ok(self.wrap_runtime_lift_success(
                    family,
                    Value::Record(Rc::new(RefCell::new(lifted))),
                ))
            }
            Value::Tuple(items) => {
                if items.is_empty() {
                    return Err(self.runtime_error(span, "lift requires at least one tuple item"));
                }
                let mut family = None;
                let mut lifted = Vec::new();
                for value in items {
                    match self.unwrap_runtime_lift_member(value, span)? {
                        RuntimeLiftMember::Success {
                            family: next_family,
                            value,
                        } => {
                            self.merge_runtime_lift_family(&mut family, next_family, span)?;
                            lifted.push(value);
                        }
                        RuntimeLiftMember::Failure {
                            family: next_family,
                            value,
                        } => {
                            self.merge_runtime_lift_family(&mut family, next_family, span)?;
                            return Ok(self.wrap_runtime_lift_failure(next_family, value));
                        }
                    }
                }
                let family = family.expect("non-empty lift established a family");
                Ok(self.wrap_runtime_lift_success(family, Value::Tuple(lifted)))
            }
            other => Err(self.runtime_error(
                span,
                format!(
                    "lift expects a shape or tuple value, got {}",
                    other.render()
                ),
            )),
        }
    }

    fn unwrap_runtime_lift_member(
        &self,
        value: Value,
        span: Option<Span>,
    ) -> Result<RuntimeLiftMember, Diagnostic> {
        let Value::Aggregate(aggregate) = value else {
            return Err(self.runtime_error(
                span,
                "lift members must be Option, Result, or Either values",
            ));
        };
        let (type_name, case_name, fields) = {
            let aggregate = aggregate.borrow();
            (
                aggregate.type_name.clone(),
                aggregate.case_name.clone(),
                aggregate.fields.clone(),
            )
        };
        let first_field = fields.first().cloned();
        match (type_name.as_str(), case_name.as_deref()) {
            ("Option", Some("Some")) => Ok(RuntimeLiftMember::Success {
                family: RuntimeLiftFamily::Option,
                value: first_field.expect("Option.Some payload"),
            }),
            ("Option", Some("None")) => Ok(RuntimeLiftMember::Failure {
                family: RuntimeLiftFamily::Option,
                value: None,
            }),
            ("Result", Some("Ok")) => Ok(RuntimeLiftMember::Success {
                family: RuntimeLiftFamily::Result,
                value: first_field.expect("Result.Ok payload"),
            }),
            ("Result", Some("Err")) => Ok(RuntimeLiftMember::Failure {
                family: RuntimeLiftFamily::Result,
                value: Some(first_field.expect("Result.Err payload")),
            }),
            ("Either", Some("Right")) => Ok(RuntimeLiftMember::Success {
                family: RuntimeLiftFamily::Either,
                value: first_field.expect("Either.Right payload"),
            }),
            ("Either", Some("Left")) => Ok(RuntimeLiftMember::Failure {
                family: RuntimeLiftFamily::Either,
                value: Some(first_field.expect("Either.Left payload")),
            }),
            _ => Err(self.runtime_error(
                span,
                format!(
                    "lift cannot unwrap {}.{}",
                    type_name,
                    case_name.unwrap_or_default()
                ),
            )),
        }
    }

    fn merge_runtime_lift_family(
        &self,
        family: &mut Option<RuntimeLiftFamily>,
        next: RuntimeLiftFamily,
        span: Option<Span>,
    ) -> Result<(), Diagnostic> {
        if let Some(current) = family {
            if *current != next {
                return Err(
                    self.runtime_error(span, "lift members must use the same wrapper family")
                );
            }
        } else {
            *family = Some(next);
        }
        Ok(())
    }

    fn wrap_runtime_lift_success(&self, family: RuntimeLiftFamily, value: Value) -> Value {
        match family {
            RuntimeLiftFamily::Option => self.option_some(value),
            RuntimeLiftFamily::Result => self.result_ok(value),
            RuntimeLiftFamily::Either => self.either_right(value),
        }
    }

    fn wrap_runtime_lift_failure(&self, family: RuntimeLiftFamily, value: Option<Value>) -> Value {
        match family {
            RuntimeLiftFamily::Option => self.option_none(),
            RuntimeLiftFamily::Result => self.result_err(value.unwrap_or(Value::Unit)),
            RuntimeLiftFamily::Either => self.either_left(value.unwrap_or(Value::Unit)),
        }
    }

    fn construct_enum_case(
        &mut self,
        explicit_enum: Option<&str>,
        case_name: &str,
        args: Vec<Value>,
        span: Option<Span>,
        from_call: bool,
    ) -> Result<Value, Diagnostic> {
        let mut matches = self
            .runtime
            .types
            .iter()
            .filter(|ty| {
                ty.kind == crate::ast::TypeKind::Enum
                    && explicit_enum.is_none_or(|name| ty.name == name)
                    && ty.enum_cases.iter().any(|case| case.name == case_name)
            })
            .collect::<Vec<_>>();
        if matches.is_empty() {
            return Err(self.runtime_error(span, format!("unknown enum case '{}'", case_name)));
        }
        if matches.len() > 1 {
            return Err(self.runtime_error(
                span,
                format!("enum case '{}' is ambiguous in this runtime", case_name),
            ));
        }
        let ty = matches.remove(0);
        let case = ty
            .enum_cases
            .iter()
            .find(|case| case.name == case_name)
            .expect("matched case");
        if from_call && case.fields.is_empty() && args.is_empty() {
            let display_name = explicit_enum
                .map(|enum_name| format!("{enum_name}.{case_name}"))
                .unwrap_or_else(|| case_name.to_string());
            return Err(self.runtime_error(
                span,
                format!(
                    "enum case '{display_name}' does not accept call syntax; use '{display_name}'"
                ),
            ));
        }
        if args.is_empty() && case.fields.iter().all(|field| field.initializer.is_some()) {
            return Ok(Value::Aggregate(Rc::new(RefCell::new(AggregateValue {
                runtime_type_id: Some(ty.id),
                type_name: ty.name.clone(),
                kind: crate::ast::TypeKind::Enum,
                case_id: Some(case.id),
                case_name: Some(case_name.to_string()),
                field_names: case.fields.iter().map(|field| field.name.clone()).collect(),
                fields: case
                    .fields
                    .iter()
                    .map(|field| self.runtime_field_default_value(field))
                    .collect(),
            }))));
        }
        let required = case
            .fields
            .iter()
            .filter(|field| field.initializer.is_none())
            .count();
        if args.len() < required || args.len() > case.fields.len() {
            return Err(self.runtime_error(
                span,
                format!(
                    "enum case '{}.{}' expects {}..{} arguments, got {}",
                    ty.name,
                    case_name,
                    required,
                    case.fields.len(),
                    args.len()
                ),
            ));
        }
        let mut field_names = Vec::with_capacity(case.fields.len());
        let mut values = Vec::with_capacity(case.fields.len());
        let mut supplied = args.into_iter().peekable();
        for (index, field) in case.fields.iter().enumerate() {
            let required_remaining = case.fields[index + 1..]
                .iter()
                .filter(|field| field.initializer.is_none())
                .count();
            let supplied_remaining = supplied.len();
            let value = if field.initializer.is_none() || supplied_remaining > required_remaining {
                supplied.next().expect("enum case arg")
            } else {
                self.runtime_field_default_value(field)
            };
            field_names.push(field.name.clone());
            values.push(value);
        }
        Ok(Value::Aggregate(Rc::new(RefCell::new(AggregateValue {
            runtime_type_id: Some(ty.id),
            type_name: ty.name.clone(),
            kind: crate::ast::TypeKind::Enum,
            case_id: Some(case.id),
            case_name: Some(case_name.to_string()),
            field_names,
            fields: values,
        }))))
    }

    fn invoke_intrinsic(
        &mut self,
        intrinsic: &ir::Intrinsic,
        args: Vec<Value>,
        span: Option<Span>,
    ) -> Result<Value, Diagnostic> {
        match intrinsic {
            ir::Intrinsic::Print => self.invoke_print(false, args, span),
            ir::Intrinsic::Println => self.invoke_print(true, args, span),
            ir::Intrinsic::Printf => self.invoke_printf(args, span),
            ir::Intrinsic::Panic => {
                let message = self.render_panic_message(&args, span)?;
                Err(self.runtime_error(span, message))
            }
            ir::Intrinsic::Assert => self.invoke_assert(args, span),
            ir::Intrinsic::Ensure => {
                if args.len() != 2 {
                    return Err(self.runtime_error(span, "ensure expects 2 arguments"));
                }
                let condition = args[0].as_bool(self, span, "ensure condition")?;
                if condition {
                    Ok(self.result_ok(Value::Unit))
                } else {
                    let error = if self.value_is_zero_arg_closure(&args[1]) {
                        self.invoke_value(args[1].clone(), Vec::new(), span)?
                    } else {
                        args[1].clone()
                    };
                    Ok(self.result_err(error))
                }
            }
            ir::Intrinsic::IterInit => {
                if args.len() != 1 {
                    return Err(self.runtime_error(span, "IterInit expects 1 argument"));
                }
                self.iter_init(args.into_iter().next().expect("iter arg"), span)
            }
            ir::Intrinsic::IterHasNext => {
                if args.len() != 1 {
                    return Err(self.runtime_error(span, "IterHasNext expects 1 argument"));
                }
                self.iter_has_next(args.into_iter().next().expect("iter arg"), span)
            }
            ir::Intrinsic::IterNext => {
                if args.len() != 1 {
                    return Err(self.runtime_error(span, "IterNext expects 1 argument"));
                }
                self.iter_next(args.into_iter().next().expect("iter arg"), span)
            }
            ir::Intrinsic::ListAppend => {
                if args.len() != 2 {
                    return Err(self.runtime_error(span, "ListAppend expects 2 arguments"));
                }
                self.list_append(args[0].clone(), args[1].clone(), span)
            }
            ir::Intrinsic::ExtractSuccessIsSet => {
                if args.len() != 1 {
                    return Err(self.runtime_error(span, "ExtractSuccessIsSet expects 1 argument"));
                }
                Ok(Value::Bool(matches!(
                    &args[0],
                    Value::Aggregate(variant)
                        if matches!(
                            variant.borrow().case_name.as_deref(),
                            Some("Some" | "Ok" | "Right")
                        )
                )))
            }
            ir::Intrinsic::ExtractSuccessValue => {
                if args.len() != 1 {
                    return Err(self.runtime_error(span, "ExtractSuccessValue expects 1 argument"));
                }
                Ok(match &args[0] {
                    Value::Aggregate(variant)
                        if matches!(
                            variant.borrow().case_name.as_deref(),
                            Some("Some" | "Ok" | "Right")
                        ) =>
                    {
                        pattern_field_value(&args[0], "value").unwrap_or(Value::Unit)
                    }
                    _ => Value::Unit,
                })
            }
            ir::Intrinsic::VariantIs(case_name) => {
                if args.len() != 1 {
                    return Err(self.runtime_error(span, "VariantIs expects 1 argument"));
                }
                Ok(Value::Bool(matches!(
                    &args[0],
                    Value::Aggregate(variant)
                        if variant.borrow().case_name.as_deref() == Some(case_name.as_str())
                )))
            }
            ir::Intrinsic::VariantField(field_name) => {
                if args.len() != 1 {
                    return Err(self.runtime_error(span, "VariantField expects 1 argument"));
                }
                Ok(pattern_field_value(&args[0], field_name).unwrap_or(Value::Unit))
            }
        }
    }

    fn invoke_os_method(
        &mut self,
        method: &str,
        args: Vec<Value>,
        span: Option<Span>,
    ) -> Result<Value, Diagnostic> {
        match method {
            "print" => self.invoke_print(false, args, span),
            "println" => self.invoke_print(true, args, span),
            "printf" => self.invoke_printf(args, span),
            "panic" => {
                let message = self.render_panic_message(&args, span)?;
                Err(self.runtime_error(span, message))
            }
            "assert" => self.invoke_assert(args, span),
            _ => Err(self.runtime_error(span, format!("unknown OS method '{}'", method))),
        }
    }

    fn invoke_print(
        &mut self,
        newline: bool,
        args: Vec<Value>,
        span: Option<Span>,
    ) -> Result<Value, Diagnostic> {
        let rendered = args
            .iter()
            .map(|value| self.render_value(value, span, "print argument"))
            .collect::<Result<Vec<_>, _>>()?
            .join(" ");
        self.output.push_str(&rendered);
        if newline {
            self.output.push('\n');
        }
        Ok(Value::Unit)
    }

    fn invoke_printf(&mut self, args: Vec<Value>, span: Option<Span>) -> Result<Value, Diagnostic> {
        if args.is_empty() {
            return Err(self.runtime_error(span, "printf expects at least 1 argument"));
        }
        let format = match &args[0] {
            Value::String(value) => value.clone(),
            other => self.render_value(other, span, "printf format")?,
        };
        for value in &args[1..] {
            self.ensure_observable_value(value, span, "printf argument")?;
        }
        let text = format_printf(&format, &args[1..])
            .map_err(|message| self.runtime_error(span, message))?;
        self.output.push_str(&text);
        Ok(Value::Unit)
    }

    fn invoke_assert(
        &mut self,
        mut args: Vec<Value>,
        span: Option<Span>,
    ) -> Result<Value, Diagnostic> {
        if !matches!(args.len(), 1 | 2) {
            return Err(self.runtime_error(
                span,
                format!("assert expects 1 or 2 arguments, got {}", args.len()),
            ));
        }
        if args[0].as_bool(self, span, "assert condition")? {
            return Ok(Value::Unit);
        }

        let panic_args = if args.len() == 2 {
            vec![args.remove(1)]
        } else {
            vec![Value::String("assert condition was false".to_string())]
        };
        let message = self.render_panic_message(&panic_args, span)?;
        Err(self.runtime_error(span, message))
    }

    fn render_panic_message(
        &self,
        args: &[Value],
        span: Option<Span>,
    ) -> Result<String, Diagnostic> {
        let message = if args.is_empty() {
            "panic".to_string()
        } else {
            args.iter()
                .map(|value| self.render_value(value, span, "panic argument"))
                .collect::<Result<String, _>>()?
        };
        Ok(if let Some(span) = span {
            format!(
                "panic: {} at {}:{}",
                message, span.start_pos.line, span.start_pos.column
            )
        } else {
            format!("panic: {}", message)
        })
    }

    fn iter_init(&mut self, value: Value, span: Option<Span>) -> Result<Value, Diagnostic> {
        match value {
            Value::Iterator(iterator) => Ok(Value::Iterator(iterator)),
            Value::List(items) => Ok(Value::Iterator(Rc::new(RefCell::new(
                IteratorState::List { items, index: 0 },
            )))),
            _ => self.invoke_method(value, "iterator", Vec::new(), span),
        }
    }

    fn iter_has_next(&mut self, value: Value, span: Option<Span>) -> Result<Value, Diagnostic> {
        match value {
            Value::Iterator(iterator) => {
                let has_next = match &*iterator.borrow() {
                    IteratorState::List { items, index } => *index < items.borrow().len(),
                    IteratorState::Range { current, end, step } => {
                        if *step >= 0 {
                            *current < *end
                        } else {
                            *current > *end
                        }
                    }
                };
                Ok(Value::Bool(has_next))
            }
            _ => Err(self.runtime_error(span, "IterHasNext expects an iterator")),
        }
    }

    fn iter_next(&mut self, value: Value, span: Option<Span>) -> Result<Value, Diagnostic> {
        match value {
            Value::Iterator(iterator) => {
                let mut iterator = iterator.borrow_mut();
                match &mut *iterator {
                    IteratorState::List { items, index } => {
                        let items = items.borrow();
                        let Some(value) = items.get(*index) else {
                            return Err(self.runtime_error(span, "iterator is exhausted"));
                        };
                        *index += 1;
                        Ok(self.clone_value(value))
                    }
                    IteratorState::Range { current, step, .. } => {
                        let value = *current;
                        *current += *step;
                        Ok(Value::Int(value))
                    }
                }
            }
            _ => Err(self.runtime_error(span, "IterNext expects an iterator")),
        }
    }

    fn list_append(
        &mut self,
        list: Value,
        value: Value,
        span: Option<Span>,
    ) -> Result<Value, Diagnostic> {
        match &list {
            Value::List(items) => {
                items.borrow_mut().push(value);
                Ok(list)
            }
            _ => Err(self.runtime_error(span, "ListAppend expects a List receiver")),
        }
    }

    pub(crate) fn invoke_method(
        &mut self,
        receiver: Value,
        method: &str,
        args: Vec<Value>,
        span: Option<Span>,
    ) -> Result<Value, Diagnostic> {
        if let Some(value) =
            self.try_invoke_runtime_method(receiver.clone(), method, args.clone(), span)?
        {
            return Ok(value);
        }

        match &receiver {
            Value::Aggregate(aggregate) if aggregate.borrow().case_name.is_some() => {
                return self.invoke_user_variant_method(receiver, method, args, span);
            }
            Value::Iterator(iterator) => {
                return self.invoke_iterator_method(receiver.clone(), iterator, method, args, span);
            }
            Value::Record(fields) => {
                if let Some(value) = lookup_named_field(&fields.borrow(), method) {
                    return self.invoke_value(value, args, span);
                }
            }
            Value::Aggregate(aggregate) => {
                let (type_name, kind, field_fallback) = {
                    let aggregate = aggregate.borrow();
                    (
                        aggregate.type_name.clone(),
                        aggregate.kind,
                        self.aggregate_field_value(&aggregate, method),
                    )
                };
                if let Some(function) =
                    self.find_method_overload_for_kind(&type_name, kind, method, &args)
                {
                    return self.call_function(function, Some(receiver), None, args, span);
                }
                if let Some(value) = field_fallback {
                    return self.invoke_value(value, args, span);
                }
            }
            _ => {}
        }

        if let Some(value) =
            self.try_invoke_universal_method(receiver.clone(), method, &args, span)?
        {
            return Ok(value);
        }

        Err(self.runtime_error(
            span,
            format!(
                "method '{}' is not available on {}",
                method,
                receiver.render()
            ),
        ))
    }

    fn try_invoke_universal_method(
        &self,
        receiver: Value,
        method: &str,
        args: &[Value],
        span: Option<Span>,
    ) -> Result<Option<Value>, Diagnostic> {
        match method {
            "toStr" => {
                if !args.is_empty() {
                    return Err(self.runtime_error(
                        span,
                        format!("toStr expects 0 arguments, got {}", args.len()),
                    ));
                }
                Ok(Some(Value::String(receiver.render())))
            }
            "equals" => {
                if args.len() != 1 {
                    return Err(self.runtime_error(
                        span,
                        format!("equals expects 1 argument, got {}", args.len()),
                    ));
                }
                Ok(Some(Value::Bool(values_equal(&receiver, &args[0]))))
            }
            _ => Ok(None),
        }
    }

    fn invoke_user_variant_method(
        &mut self,
        receiver: Value,
        method: &str,
        args: Vec<Value>,
        span: Option<Span>,
    ) -> Result<Value, Diagnostic> {
        let Value::Aggregate(variant) = &receiver else {
            unreachable!();
        };
        let (type_name, case_name) = {
            let variant = variant.borrow();
            (
                variant.type_name.clone(),
                variant
                    .case_name
                    .clone()
                    .unwrap_or_else(|| "<unknown>".to_string()),
            )
        };
        if let Some(function) = self.find_method_overload_for_kind(
            &type_name,
            crate::ast::TypeKind::Enum,
            method,
            &args,
        ) {
            return self.call_function(function, Some(receiver), None, args, span);
        }
        if let Some(value) =
            self.try_invoke_universal_method(receiver.clone(), method, &args, span)?
        {
            return Ok(value);
        }
        Err(self.runtime_error(
            span,
            format!(
                "method '{}' is not available on variant '{}.{}'",
                method, type_name, case_name
            ),
        ))
    }

    fn invoke_iterator_method(
        &mut self,
        receiver: Value,
        iterator: &Rc<RefCell<IteratorState>>,
        method: &str,
        args: Vec<Value>,
        span: Option<Span>,
    ) -> Result<Value, Diagnostic> {
        match method {
            "hasNext" => {
                if !args.is_empty() {
                    return Err(self.runtime_error(span, "Iterator.hasNext expects 0 arguments"));
                }
                self.iter_has_next(Value::Iterator(iterator.clone()), span)
            }
            "next" => {
                if !args.is_empty() {
                    return Err(self.runtime_error(span, "Iterator.next expects 0 arguments"));
                }
                self.iter_next(receiver, span)
            }
            "zip" => {
                let [other] = args.as_slice() else {
                    return Err(self.runtime_error(span, "Iterator.zip expects 1 argument"));
                };
                let lhs = iterator_values(iterator, span, self)?;
                let rhs = iterable_values(other.clone(), span, self)?;
                let out = lhs
                    .into_iter()
                    .zip(rhs)
                    .map(|(left, right)| Value::Tuple(vec![left, right]))
                    .collect();
                Ok(Value::List(Rc::new(RefCell::new(out))))
            }
            "zipWithIndex" => {
                if !args.is_empty() {
                    return Err(
                        self.runtime_error(span, "Iterator.zipWithIndex expects 0 arguments")
                    );
                }
                let out = iterator_values(iterator, span, self)?
                    .into_iter()
                    .enumerate()
                    .map(|(index, value)| Value::Tuple(vec![value, Value::Int(index as i64)]))
                    .collect();
                Ok(Value::List(Rc::new(RefCell::new(out))))
            }
            _ => Err(self.runtime_error(span, format!("unsupported Iterator method '{}'", method))),
        }
    }

    fn lookup_runtime_value(&self, frame: Option<&Frame>, name: &str) -> Option<Value> {
        frame
            .and_then(|frame| self.lookup_local_by_name(frame, name))
            .or_else(|| self.lookup_global_by_name(name))
    }

    fn lookup_local_by_name(&self, frame: &Frame, name: &str) -> Option<Value> {
        self.program
            .function(frame.function)?
            .locals
            .iter()
            .find(|local| local.name == name)
            .and_then(|local| frame.locals.get(local.id.0).cloned())
    }

    fn lookup_global_by_name(&self, name: &str) -> Option<Value> {
        self.program
            .globals
            .iter()
            .find(|global| global.name == name)
            .and_then(|global| self.globals.get(global.id.0).cloned())
    }

    fn lookup_function(&self, name: &str) -> Option<ir::FunctionId> {
        self.program
            .functions
            .iter()
            .find(|function| function.name == name)
            .map(|function| function.id)
    }

    fn lookup_type_by_kind(
        &self,
        name: &str,
        kind: crate::ast::TypeKind,
    ) -> Option<&runtime::RuntimeType> {
        self.runtime.type_by_name_kind(name, kind)
    }

    fn lookup_singleton(
        &mut self,
        name: &str,
        span: Option<Span>,
    ) -> Result<Option<Value>, Diagnostic> {
        let Some(ty) = self
            .lookup_type_by_kind(name, crate::ast::TypeKind::Single)
            .cloned()
        else {
            return Ok(None);
        };
        if let Some(existing) = &self.singletons[ty.id.0] {
            return Ok(Some(existing.clone()));
        }
        let field_values = self.allocate_runtime_fields(&ty.fields);
        let value = Value::Aggregate(Rc::new(RefCell::new(AggregateValue {
            runtime_type_id: Some(ty.id),
            type_name: ty.name.clone(),
            kind: ty.kind,
            case_id: None,
            case_name: None,
            field_names: ty.fields.iter().map(|field| field.name.clone()).collect(),
            fields: field_values,
        })));
        if let Some(field_init) = ty.field_init {
            let _ = self.call_function(field_init, Some(value.clone()), None, Vec::new(), span)?;
        }
        if let Some(init) =
            self.find_method_overload_for_kind(&ty.name, crate::ast::TypeKind::Single, "new", &[])
        {
            let _ = self.call_function(init, Some(value.clone()), None, Vec::new(), span)?;
        }
        self.singletons[ty.id.0] = Some(value.clone());
        Ok(Some(value))
    }

    fn find_method_overload_for_kind(
        &self,
        owner: &str,
        kind: crate::ast::TypeKind,
        method: &str,
        args: &[Value],
    ) -> Option<ir::FunctionId> {
        let mut visited = HashSet::new();
        let mut candidates = Vec::new();
        self.collect_methods_for_kind_inner(owner, kind, method, &mut visited, &mut candidates);
        self.choose_function_overload(&candidates, args)
    }

    fn collect_methods_for_kind_inner(
        &self,
        owner: &str,
        kind: crate::ast::TypeKind,
        method: &str,
        visited: &mut HashSet<(String, crate::ast::TypeKind)>,
        out: &mut Vec<ir::FunctionId>,
    ) {
        if !visited.insert((owner.to_string(), kind)) {
            return;
        }
        let Some(ty) = self.lookup_type_by_kind(owner, kind) else {
            return;
        };
        out.extend(self.runtime.methods_named(ty.id, method));
        for bound in &ty.with_bounds {
            let Some(bound_ty) = self.runtime.type_by_id(*bound) else {
                continue;
            };
            self.collect_methods_for_kind_inner(
                &bound_ty.name,
                bound_ty.kind,
                method,
                visited,
                out,
            );
        }
    }

    fn choose_function_overload(
        &self,
        candidates: &[ir::FunctionId],
        args: &[Value],
    ) -> Option<ir::FunctionId> {
        let mut best = None;
        let mut best_score = i32::MIN;
        for candidate in candidates {
            let Some(function) = self.program.function(*candidate) else {
                continue;
            };
            if let Some(variadic_index) = function.param_variadic.iter().position(|value| *value) {
                if args.len() < variadic_index
                    && !function.param_defaults[args.len()..variadic_index]
                        .iter()
                        .all(Option::is_some)
                {
                    continue;
                }
                let Some(variadic_elem) = self.variadic_element_type(function, variadic_index)
                else {
                    continue;
                };
                let mut score = 9 + args.len() as i32;
                let mut matches = true;
                let packed_variadic = if args.len() == function.params.len() {
                    function
                        .params
                        .get(variadic_index)
                        .and_then(|param| function.locals.get(param.0))
                        .is_some_and(|local| {
                            self.value_matches_type(&args[variadic_index], &local.ty)
                        })
                } else {
                    false
                };
                for (index, arg) in args.iter().enumerate() {
                    let param_ty = if index < variadic_index {
                        let Some(local) = function
                            .params
                            .get(index)
                            .and_then(|param| function.locals.get(param.0))
                        else {
                            matches = false;
                            break;
                        };
                        &local.ty
                    } else if packed_variadic && index == variadic_index {
                        let Some(local) = function
                            .params
                            .get(index)
                            .and_then(|param| function.locals.get(param.0))
                        else {
                            matches = false;
                            break;
                        };
                        &local.ty
                    } else {
                        variadic_elem
                    };
                    if !self.value_matches_type(arg, param_ty) {
                        matches = false;
                        break;
                    }
                    if !matches!(param_ty, ir::Type::Unknown | ir::Type::TypeParam(_)) {
                        score += 2;
                    }
                }
                if matches && score > best_score {
                    best = Some(*candidate);
                    best_score = score;
                }
                continue;
            }
            let default_suffix_matches = args.len() <= function.params.len()
                && function.param_defaults[args.len()..]
                    .iter()
                    .all(Option::is_some);
            let mut score = if function.params.len() == args.len() {
                10
            } else if default_suffix_matches {
                8
            } else if function.params.len() == 1 && args.len() > 1 {
                let Some(local) = function.locals.get(function.params[0].0) else {
                    continue;
                };
                let ir::Type::Tuple(items) = &local.ty else {
                    continue;
                };
                if items.len() != args.len()
                    || !args
                        .iter()
                        .zip(items)
                        .all(|(arg, ty)| self.value_matches_type(arg, ty))
                {
                    continue;
                }
                5 + 2 * args.len() as i32
            } else {
                continue;
            };

            if function.params.len() >= args.len() {
                for (param, arg) in function.params.iter().take(args.len()).zip(args) {
                    let Some(local) = function.locals.get(param.0) else {
                        continue;
                    };
                    if self.value_matches_type(arg, &local.ty) {
                        score += 2;
                    }
                }
            }
            if score > best_score {
                best = Some(*candidate);
                best_score = score;
            }
        }
        best
    }

    fn get_member(&self, base: Value, name: &str, span: Option<Span>) -> Result<Value, Diagnostic> {
        if name == "runtimeType" {
            return Ok(Value::RuntimeType(self.runtime_type_value_for_value(&base)));
        }

        match base {
            Value::Aggregate(aggregate) => {
                if matches!(name, "stdout" | "stderr")
                    && aggregate.borrow().type_name == "OS"
                    && aggregate.borrow().case_name.is_none()
                {
                    return Ok(Value::Aggregate(aggregate.clone()));
                }
                let aggregate = aggregate.borrow();
                self.aggregate_field_value(&aggregate, name).ok_or_else(|| {
                    self.runtime_error(
                        span,
                        if let Some(case_name) = &aggregate.case_name {
                            format!(
                                "variant '{}.{}' has no field '{}'",
                                aggregate.type_name, case_name, name
                            )
                        } else {
                            format!("value '{}' has no field '{}'", aggregate.type_name, name)
                        },
                    )
                })
            }
            Value::Record(fields) => lookup_named_field(&fields.borrow(), name)
                .ok_or_else(|| self.runtime_error(span, format!("shape has no field '{}'", name))),
            Value::Tuple(items) => tuple_member(&items, name)
                .ok_or_else(|| self.runtime_error(span, format!("tuple has no member '{}'", name))),
            _ => Err(self.runtime_error(
                span,
                format!("cannot access field '{}' on {}", name, base.render()),
            )),
        }
    }

    fn aggregate_field_value(&self, aggregate: &AggregateValue, name: &str) -> Option<Value> {
        aggregate
            .field_names
            .iter()
            .position(|field_name| field_name == name)
            .and_then(|index| aggregate.fields.get(index).cloned())
            .or_else(|| {
                self.aggregate_visible_field_index(aggregate, name)
                    .and_then(|index| aggregate.fields.get(index).cloned())
            })
    }

    fn aggregate_visible_field_index(
        &self,
        aggregate: &AggregateValue,
        name: &str,
    ) -> Option<usize> {
        let visible_index = ordered_member_index(name)?;
        let type_id = aggregate.runtime_type_id?;
        let runtime_type = self.runtime.types.get(type_id.0)?;
        if let Some(case_id) = aggregate.case_id {
            return runtime_type
                .enum_cases
                .get(case_id.0)?
                .fields
                .iter()
                .filter(|field| !field.hidden)
                .nth(visible_index)
                .map(|field| field.slot.0);
        }
        runtime_type
            .fields
            .iter()
            .filter(|field| !field.hidden)
            .nth(visible_index)
            .map(|field| field.slot.0)
    }

    fn set_member(
        &mut self,
        base: Value,
        name: &str,
        value: Value,
        span: Option<Span>,
    ) -> Result<(), Diagnostic> {
        match base {
            Value::Aggregate(instance) if instance.borrow().case_name.is_none() => {
                let mut instance = instance.borrow_mut();
                if let Some(index) = instance.field_names.iter().position(|field| field == name) {
                    instance.fields[index] = value;
                    Ok(())
                } else {
                    Err(self.runtime_error(span, format!("field '{}' does not exist", name)))
                }
            }
            Value::Record(fields) => set_named_field(&mut fields.borrow_mut(), name, value)
                .ok_or_else(|| {
                    self.runtime_error(span, format!("shape field '{}' does not exist", name))
                }),
            _ => Err(self.runtime_error(
                span,
                format!("cannot assign field '{}' on {}", name, base.render()),
            )),
        }
    }

    fn index_value(
        &mut self,
        base: Value,
        index: Value,
        span: Option<Span>,
    ) -> Result<Value, Diagnostic> {
        if let Value::Map(entries) = base {
            self.ensure_observable_value(&index, span, "map lookup key")?;
            let mut value = None;
            for (key, entry_value) in entries.borrow().iter() {
                self.ensure_observable_value(key, span, "map lookup key")?;
                if values_equal(key, &index) {
                    value = Some(entry_value.clone());
                    break;
                }
            }
            return Ok(match value {
                Some(value) => self.option_some(value),
                None => self.option_none(),
            });
        }
        let index = index.as_int(self, span, "index")?;
        match base {
            Value::List(items) => {
                let items = items.borrow();
                let index = normalize_index(items.len(), index).ok_or_else(|| {
                    self.runtime_error(span, format!("list index {} out of bounds", index))
                })?;
                items.get(index).map_or_else(
                    || Err(self.runtime_error(span, format!("list index {} out of bounds", index))),
                    |value| Ok(self.clone_value(value)),
                )
            }
            Value::Tuple(items) => {
                let index = normalize_index(items.len(), index).ok_or_else(|| {
                    self.runtime_error(span, format!("tuple index {} out of bounds", index))
                })?;
                items.get(index).map_or_else(
                    || Err(self.runtime_error(span, format!("tuple index {} out of bounds", index))),
                    |value| Ok(self.clone_value(value)),
                )
            }
            other => self.invoke_method(other, "[]", vec![Value::Int(index)], span),
        }
    }

    fn set_index(
        &mut self,
        base: Value,
        index: Value,
        value: Value,
        span: Option<Span>,
    ) -> Result<(), Diagnostic> {
        if let Value::Map(entries) = base {
            map_put_entry(&mut entries.borrow_mut(), index, value);
            return Ok(());
        }
        let index = index.as_int(self, span, "index")?;
        match base {
            Value::List(items) => {
                let mut items = items.borrow_mut();
                let index = normalize_index(items.len(), index).ok_or_else(|| {
                    self.runtime_error(span, format!("list index {} out of bounds", index))
                })?;
                let Some(slot) = items.get_mut(index) else {
                    return Err(
                        self.runtime_error(span, format!("list index {} out of bounds", index))
                    );
                };
                *slot = value;
                Ok(())
            }
            _ => Err(self.runtime_error(span, format!("cannot assign index on {}", base.render()))),
        }
    }

    fn eval_unary(
        &mut self,
        op: ir::UnaryOp,
        operand: Value,
        span: Option<Span>,
    ) -> Result<Value, Diagnostic> {
        match op {
            ir::UnaryOp::Neg => match operand {
                Value::Int(value) => Ok(Value::Int(-value)),
                Value::Float(value) => Ok(Value::Float(-value)),
                other => self.invoke_method(other, "-", Vec::new(), span),
            },
            ir::UnaryOp::Not => Ok(Value::Bool(!operand.as_bool(self, span, "logical not")?)),
        }
    }

    fn eval_binary(
        &mut self,
        op: ir::BinaryOp,
        left: Value,
        right: Value,
        span: Option<Span>,
    ) -> Result<Value, Diagnostic> {
        match op {
            ir::BinaryOp::Add => match (&left, &right) {
                (Value::Int(lhs), Value::Int(rhs)) => Ok(Value::Int(lhs + rhs)),
                (Value::Float(lhs), Value::Float(rhs)) => Ok(Value::Float(lhs + rhs)),
                (Value::Int(lhs), Value::Float(rhs)) => Ok(Value::Float(*lhs as f64 + rhs)),
                (Value::Float(lhs), Value::Int(rhs)) => Ok(Value::Float(lhs + *rhs as f64)),
                (Value::String(_), _) | (_, Value::String(_)) => Ok(Value::String(format!(
                    "{}{}",
                    left.render(),
                    right.render()
                ))),
                _ => self.invoke_method(left, "+", vec![right], span),
            },
            ir::BinaryOp::Sub => numeric_binary_or_method(
                left,
                right,
                span,
                "-",
                |lhs, rhs| lhs - rhs,
                |lhs, rhs| lhs - rhs,
                self,
            ),
            ir::BinaryOp::Mul => numeric_binary_or_method(
                left,
                right,
                span,
                "*",
                |lhs, rhs| lhs * rhs,
                |lhs, rhs| lhs * rhs,
                self,
            ),
            ir::BinaryOp::Div => numeric_binary_or_method(
                left,
                right,
                span,
                "/",
                |lhs, rhs| lhs / rhs,
                |lhs, rhs| lhs / rhs,
                self,
            ),
            ir::BinaryOp::Mod => match (left, right) {
                (Value::Int(lhs), Value::Int(rhs)) => Ok(Value::Int(lhs % rhs)),
                (left, right) => self.invoke_method(left, "%", vec![right], span),
            },
            ir::BinaryOp::Eq => {
                self.ensure_observable_value(&left, span, "equality comparison")?;
                self.ensure_observable_value(&right, span, "equality comparison")?;
                Ok(Value::Bool(values_equal(&left, &right)))
            }
            ir::BinaryOp::NotEq => {
                self.ensure_observable_value(&left, span, "equality comparison")?;
                self.ensure_observable_value(&right, span, "equality comparison")?;
                Ok(Value::Bool(!values_equal(&left, &right)))
            }
            ir::BinaryOp::Less => compare_binary(left, right, span, |lhs, rhs| lhs < rhs, self),
            ir::BinaryOp::LessEq => compare_binary(left, right, span, |lhs, rhs| lhs <= rhs, self),
            ir::BinaryOp::Greater => compare_binary(left, right, span, |lhs, rhs| lhs > rhs, self),
            ir::BinaryOp::GreaterEq => {
                compare_binary(left, right, span, |lhs, rhs| lhs >= rhs, self)
            }
            ir::BinaryOp::And => Ok(Value::Bool(
                left.as_bool(self, span, "left side of &&")?
                    && right.as_bool(self, span, "right side of &&")?,
            )),
            ir::BinaryOp::Or => Ok(Value::Bool(
                left.as_bool(self, span, "left side of ||")?
                    || right.as_bool(self, span, "right side of ||")?,
            )),
        }
    }

    fn switch_matches(&self, value: &Value, arm: &ir::SwitchValue) -> bool {
        match arm {
            ir::SwitchValue::Bool(expected) => {
                matches!(value, Value::Bool(actual) if actual == expected)
            }
            ir::SwitchValue::Int(expected) => {
                matches!(value, Value::Int(actual) if actual == expected)
            }
            ir::SwitchValue::String(expected) => {
                matches!(value, Value::String(actual) if actual == expected)
            }
            ir::SwitchValue::EnumCase(expected) => {
                matches!(
                    value,
                    Value::Aggregate(aggregate)
                        if aggregate.borrow().case_name.as_deref() == Some(expected.as_str())
                )
            }
        }
    }

    fn value_matches_type(&self, value: &Value, ty: &ir::Type) -> bool {
        match ty {
            ir::Type::Unknown => true,
            ir::Type::Never => false,
            ir::Type::Unit => matches!(value, Value::Unit),
            ir::Type::Bool => matches!(value, Value::Bool(_)),
            ir::Type::Int => matches!(value, Value::Int(_)),
            ir::Type::Float => matches!(value, Value::Float(_)),
            ir::Type::Str => matches!(value, Value::String(_)),
            ir::Type::Named { name, .. } => match value {
                Value::List(_) => name == "List" || name == "Array",
                Value::Set(_) => name == "Set",
                Value::Map(_) => name == "Map",
                Value::Iterator(_) => name == "Iterator" || name == "IntRange",
                Value::Aggregate(aggregate) => {
                    let aggregate = aggregate.borrow();
                    if aggregate.kind == crate::ast::TypeKind::Enum {
                        aggregate.type_name == *name
                    } else {
                        self.aggregate_matches_named_type(
                            &aggregate.type_name,
                            aggregate.kind,
                            name,
                        )
                    }
                }
                Value::String(_) => name == "Str",
                Value::Rune(_) => name == "Rune",
                Value::Int(_) => name == "Int" || name == "Int32",
                Value::Float(_) => name == "Float" || name == "Float32",
                Value::Bool(_) => name == "Bool",
                Value::Unit => name == "Unit",
                Value::RuntimeType(runtime_type) => match name.as_str() {
                    "Type" | "Annotated" => true,
                    "ClassType" => self.runtime_type_kind_case(runtime_type) == "Class",
                    "ShapeType" => self.runtime_type_kind_case(runtime_type) == "Shape",
                    "EnumType" => self.runtime_type_kind_case(runtime_type) == "Enum",
                    "InterfaceType" => self.runtime_type_kind_case(runtime_type) == "Interface",
                    "SingleType" => self.runtime_type_kind_case(runtime_type) == "Single",
                    "AnnotationType" => self.runtime_type_kind_case(runtime_type) == "Annotation",
                    _ => false,
                },
                Value::RuntimeField { .. } => name == "Field" || name == "Annotated",
                Value::RuntimeMethod { .. } => name == "Method" || name == "Annotated",
                Value::RuntimeParam { .. } => name == "Param",
                Value::RuntimeEnumCase { .. } => name == "EnumCase" || name == "Annotated",
                other => self
                    .runtime
                    .type_by_name_kind(name, crate::ast::TypeKind::Record)
                    .is_some_and(|ty| self.value_matches_runtime_shape(other, ty)),
            },
            ir::Type::Tuple(items) => match value {
                Value::Tuple(values) => {
                    values.len() == items.len()
                        && values
                            .iter()
                            .zip(items)
                            .all(|(value, ty)| self.value_matches_type(value, ty))
                }
                _ => false,
            },
            ir::Type::Record(fields) => match value {
                Value::Record(record) => fields.iter().all(|field| {
                    lookup_named_field(&record.borrow(), &field.name)
                        .is_some_and(|value| self.value_matches_type(&value, &field.ty))
                }),
                Value::Tuple(values) => {
                    values.len() == fields.len()
                        && values
                            .iter()
                            .zip(fields)
                            .all(|(value, field)| self.value_matches_type(value, &field.ty))
                }
                Value::Aggregate(aggregate) => {
                    let aggregate = aggregate.borrow();
                    fields.iter().all(|field| {
                        self.aggregate_visible_named_field_value(&aggregate, &field.name)
                            .is_some_and(|value| self.value_matches_type(&value, &field.ty))
                    })
                }
                _ => false,
            },
            ir::Type::Function { .. } => matches!(value, Value::Closure(_)),
            ir::Type::TypeParam(_) => true,
        }
    }

    fn value_matches_runtime_shape(&self, value: &Value, ty: &runtime::RuntimeType) -> bool {
        let visible_fields = ty
            .fields
            .iter()
            .filter(|field| !field.hidden)
            .collect::<Vec<_>>();
        match value {
            Value::Aggregate(aggregate) => {
                let aggregate = aggregate.borrow();
                if aggregate.type_name == ty.name && aggregate.kind == crate::ast::TypeKind::Record
                {
                    return true;
                }
                visible_fields.iter().all(|field| {
                    self.aggregate_visible_named_field_value(&aggregate, &field.name)
                        .is_some_and(|value| self.value_matches_type(&value, &field.ty))
                })
            }
            Value::Record(record) => {
                let record = record.borrow();
                visible_fields.iter().all(|field| {
                    lookup_named_field(&record, &field.name)
                        .is_some_and(|value| self.value_matches_type(&value, &field.ty))
                })
            }
            Value::Tuple(items) => {
                items.len() == visible_fields.len()
                    && items
                        .iter()
                        .zip(visible_fields.iter())
                        .all(|(value, field)| self.value_matches_type(value, &field.ty))
            }
            _ => false,
        }
    }

    fn aggregate_visible_named_field_value(
        &self,
        aggregate: &AggregateValue,
        name: &str,
    ) -> Option<Value> {
        let type_id = aggregate.runtime_type_id?;
        let runtime_type = self.runtime.type_by_id(type_id)?;
        let fields = if let Some(case_id) = aggregate.case_id {
            &runtime_type.enum_cases.get(case_id.0)?.fields
        } else {
            &runtime_type.fields
        };
        let field = fields
            .iter()
            .find(|field| !field.hidden && field.name == name)?;
        aggregate.fields.get(field.slot.0).cloned()
    }

    fn aggregate_from_shape_values(&self, ty: &runtime::RuntimeType, values: Vec<Value>) -> Value {
        let visible_fields = ty
            .fields
            .iter()
            .filter(|field| !field.hidden)
            .collect::<Vec<_>>();
        let mut fields = self.allocate_runtime_fields(&ty.fields);
        for (value, field) in values.into_iter().zip(visible_fields) {
            fields[field.slot.0] = self.coerce_value_to_type(value, &field.ty);
        }
        Value::Aggregate(Rc::new(RefCell::new(AggregateValue {
            runtime_type_id: Some(ty.id),
            type_name: ty.name.clone(),
            kind: ty.kind,
            case_id: None,
            case_name: None,
            field_names: ty.fields.iter().map(|field| field.name.clone()).collect(),
            fields,
        })))
    }

    fn coerce_value_to_named_shape(
        &self,
        value: Value,
        ty: &runtime::RuntimeType,
    ) -> Option<Value> {
        let visible_fields = ty
            .fields
            .iter()
            .filter(|field| !field.hidden)
            .collect::<Vec<_>>();
        match value {
            Value::Aggregate(aggregate) => {
                let aggregate_ref = aggregate.borrow();
                if aggregate_ref.type_name == ty.name
                    && aggregate_ref.kind == crate::ast::TypeKind::Record
                {
                    return Some(Value::Aggregate(aggregate.clone()));
                }
                let values = visible_fields
                    .iter()
                    .map(|field| {
                        self.aggregate_visible_named_field_value(&aggregate_ref, &field.name)
                    })
                    .collect::<Option<Vec<_>>>()?;
                Some(self.aggregate_from_shape_values(ty, values))
            }
            Value::Record(record) => {
                let record_ref = record.borrow();
                let values = visible_fields
                    .iter()
                    .map(|field| lookup_named_field(&record_ref, &field.name))
                    .collect::<Option<Vec<_>>>()?;
                Some(self.aggregate_from_shape_values(ty, values))
            }
            Value::Tuple(items) if items.len() == visible_fields.len() => {
                Some(self.aggregate_from_shape_values(ty, items))
            }
            other => Some(other).filter(|value| self.value_matches_runtime_shape(value, ty)),
        }
    }

    fn coerce_value_to_type(&self, value: Value, ty: &ir::Type) -> Value {
        match ty {
            ir::Type::Named { name, .. } => self
                .runtime
                .type_by_name_kind(name, crate::ast::TypeKind::Record)
                .and_then(|ty| self.coerce_value_to_named_shape(value.clone(), ty))
                .unwrap_or(value),
            ir::Type::Record(fields) => match value {
                Value::Tuple(items) if items.len() == fields.len() => {
                    Value::Record(Rc::new(RefCell::new(
                        fields
                            .iter()
                            .zip(items)
                            .map(|(field, value)| {
                                (
                                    field.name.clone(),
                                    self.coerce_value_to_type(value, &field.ty),
                                )
                            })
                            .collect(),
                    )))
                }
                Value::Record(record) => {
                    let values = record.borrow();
                    if fields
                        .iter()
                        .all(|field| lookup_named_field(&values, &field.name).is_some())
                    {
                        Value::Record(Rc::new(RefCell::new(
                            fields
                                .iter()
                                .map(|field| {
                                    (
                                        field.name.clone(),
                                        self.coerce_value_to_type(
                                            lookup_named_field(&values, &field.name)
                                                .expect("shape field lookup"),
                                            &field.ty,
                                        ),
                                    )
                                })
                                .collect(),
                        )))
                    } else if values.len() == fields.len() {
                        Value::Record(Rc::new(RefCell::new(
                            fields
                                .iter()
                                .zip(values.iter())
                                .map(|(field, (_, value))| {
                                    (
                                        field.name.clone(),
                                        self.coerce_value_to_type(value.clone(), &field.ty),
                                    )
                                })
                                .collect(),
                        )))
                    } else {
                        Value::Record(record.clone())
                    }
                }
                Value::Aggregate(aggregate) => {
                    let aggregate_ref = aggregate.borrow();
                    if fields.iter().all(|field| {
                        self.aggregate_visible_named_field_value(&aggregate_ref, &field.name)
                            .is_some()
                    }) {
                        Value::Record(Rc::new(RefCell::new(
                            fields
                                .iter()
                                .filter_map(|field| {
                                    self.aggregate_visible_named_field_value(
                                        &aggregate_ref,
                                        &field.name,
                                    )
                                    .map(|value| {
                                        (
                                            field.name.clone(),
                                            self.coerce_value_to_type(value, &field.ty),
                                        )
                                    })
                                })
                                .collect(),
                        )))
                    } else {
                        Value::Aggregate(aggregate.clone())
                    }
                }
                other => other,
            },
            _ => value,
        }
    }

    fn aggregate_matches_named_type(
        &self,
        type_name: &str,
        kind: crate::ast::TypeKind,
        expected: &str,
    ) -> bool {
        if type_name == expected {
            return true;
        }
        let mut visited = HashSet::new();
        self.type_satisfies_named(type_name, kind, expected, &mut visited)
    }

    fn type_satisfies_named(
        &self,
        type_name: &str,
        kind: crate::ast::TypeKind,
        expected: &str,
        visited: &mut HashSet<(String, crate::ast::TypeKind)>,
    ) -> bool {
        if !visited.insert((type_name.to_string(), kind)) {
            return false;
        }
        let Some(ty) = self.lookup_type_by_kind(type_name, kind) else {
            return false;
        };
        ty.with_bounds.iter().any(|bound| {
            let Some(bound_ty) = self.runtime.type_by_id(*bound) else {
                return false;
            };
            bound_ty.name == expected
                || self.type_satisfies_named(&bound_ty.name, bound_ty.kind, expected, visited)
        })
    }

    pub(crate) fn runtime_error(
        &self,
        span: Option<Span>,
        message: impl Into<String>,
    ) -> Diagnostic {
        Diagnostic::error(
            "runtime_error",
            message.into(),
            span.unwrap_or_else(default_span),
        )
    }

    pub(crate) fn clone_value(&self, value: &Value) -> Value {
        value.clone()
    }

    pub(crate) fn clone_values(&self, values: &[Value]) -> Vec<Value> {
        values.iter().map(|value| self.clone_value(value)).collect()
    }

    pub(crate) fn ensure_observable_value(
        &self,
        value: &Value,
        span: Option<Span>,
        context: &str,
    ) -> Result<(), Diagnostic> {
        match value {
            Value::Tuple(items) => {
                for item in items {
                    self.ensure_observable_value(item, span, context)?;
                }
            }
            Value::List(items) | Value::Set(items) => {
                for item in items.borrow().iter() {
                    self.ensure_observable_value(item, span, context)?;
                }
            }
            Value::Map(entries) => {
                for (key, value) in entries.borrow().iter() {
                    self.ensure_observable_value(key, span, context)?;
                    self.ensure_observable_value(value, span, context)?;
                }
            }
            Value::Record(fields) => {
                for (_, value) in fields.borrow().iter() {
                    self.ensure_observable_value(value, span, context)?;
                }
            }
            Value::Aggregate(aggregate) => {
                for value in aggregate.borrow().fields.iter() {
                    self.ensure_observable_value(value, span, context)?;
                }
            }
            Value::Iterator(iterator) => {
                for value in iterator_values(iterator, span, self)? {
                    self.ensure_observable_value(&value, span, context)?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    pub(crate) fn render_value(
        &self,
        value: &Value,
        span: Option<Span>,
        context: &str,
    ) -> Result<String, Diagnostic> {
        self.ensure_observable_value(value, span, context)?;
        Ok(value.render())
    }
}

impl Value {
    pub(crate) fn as_bool(
        &self,
        in_: &Interpreter<'_>,
        span: Option<Span>,
        context: &str,
    ) -> Result<bool, Diagnostic> {
        match self {
            Value::Bool(value) => Ok(*value),
            _ => Err(in_.runtime_error(
                span,
                format!("{context} expects Bool, got {}", self.render()),
            )),
        }
    }

    pub(crate) fn as_int(
        &self,
        in_: &Interpreter<'_>,
        span: Option<Span>,
        context: &str,
    ) -> Result<i64, Diagnostic> {
        match self {
            Value::Int(value) => Ok(*value),
            _ => Err(in_.runtime_error(
                span,
                format!("{context} expects Int, got {}", self.render()),
            )),
        }
    }

    pub(crate) fn as_number(
        &self,
        in_: &Interpreter<'_>,
        span: Option<Span>,
        context: &str,
    ) -> Result<f64, Diagnostic> {
        match self {
            Value::Int(value) => Ok(*value as f64),
            Value::Float(value) => Ok(*value),
            _ => Err(in_.runtime_error(
                span,
                format!("{context} expects numeric value, got {}", self.render()),
            )),
        }
    }
}

fn lookup_named_field(fields: &[(String, Value)], name: &str) -> Option<Value> {
    fields
        .iter()
        .find(|(field_name, _)| field_name == name)
        .map(|(_, value)| value.clone())
        .or_else(|| {
            ordered_member(fields.len(), name)
                .and_then(|index| fields.get(index).map(|(_, value)| value.clone()))
        })
}

fn set_named_field(fields: &mut [(String, Value)], name: &str, value: Value) -> Option<()> {
    let field = fields
        .iter_mut()
        .find(|(field_name, _)| field_name == name)?;
    field.1 = value;
    Some(())
}

fn tuple_member(items: &[Value], name: &str) -> Option<Value> {
    ordered_member(items.len(), name).and_then(|index| items.get(index).cloned())
}

fn ordered_member_index(name: &str) -> Option<usize> {
    let index = name.strip_prefix('_')?.parse::<usize>().ok()?;
    index.checked_sub(1)
}

fn ordered_member(len: usize, name: &str) -> Option<usize> {
    let index = ordered_member_index(name)?;
    (index < len).then_some(index)
}

fn normalize_index(len: usize, index: i64) -> Option<usize> {
    if index >= 0 {
        let index = index as usize;
        (index < len).then_some(index)
    } else {
        let offset = (-index) as usize;
        (offset <= len).then_some(len - offset)
    }
}

fn aggregate_named_field(aggregate: &AggregateValue, name: &str) -> Option<Value> {
    aggregate
        .field_names
        .iter()
        .position(|field_name| field_name == name)
        .and_then(|index| aggregate.fields.get(index).cloned())
        .or_else(|| {
            ordered_member(aggregate.fields.len(), name)
                .and_then(|index| aggregate.fields.get(index).cloned())
        })
}

fn pattern_field_value(value: &Value, name: &str) -> Option<Value> {
    match value {
        Value::Aggregate(aggregate) => aggregate_named_field(&aggregate.borrow(), name),
        Value::Record(fields) => lookup_named_field(&fields.borrow(), name),
        Value::Tuple(items) => tuple_member(items, name),
        _ => None,
    }
}

pub(crate) fn push_unique(items: &mut Vec<Value>, value: Value) {
    if !items.iter().any(|existing| values_equal(existing, &value)) {
        items.push(value);
    }
}

pub(crate) fn unique_values(items: Vec<Value>) -> Vec<Value> {
    let mut out = Vec::new();
    for value in items {
        push_unique(&mut out, value);
    }
    out
}

pub(crate) fn map_put_entry(entries: &mut Vec<(Value, Value)>, key: Value, value: Value) {
    if let Some((_, slot)) = entries
        .iter_mut()
        .find(|(existing, _)| values_equal(existing, &key))
    {
        *slot = value;
    } else {
        entries.push((key, value));
    }
}

fn iterator_values(
    iterator: &Rc<RefCell<IteratorState>>,
    _span: Option<Span>,
    in_: &Interpreter<'_>,
) -> Result<Vec<Value>, Diagnostic> {
    let mut state = iterator.borrow().clone();
    let mut out = Vec::new();
    loop {
        match &mut state {
            IteratorState::List { items, index } => {
                let items = items.borrow();
                let Some(value) = items.get(*index) else {
                    break;
                };
                *index += 1;
                out.push(in_.clone_value(value));
            }
            IteratorState::Range { current, end, step } => {
                let done = if *step >= 0 {
                    *current >= *end
                } else {
                    *current <= *end
                };
                if done {
                    break;
                }
                let value = *current;
                *current += *step;
                out.push(Value::Int(value));
            }
        }
    }
    Ok(out)
}

pub(crate) fn iterable_values(
    value: Value,
    span: Option<Span>,
    in_: &Interpreter<'_>,
) -> Result<Vec<Value>, Diagnostic> {
    match value {
        Value::List(items) => {
            let items = items.borrow();
            Ok(in_.clone_values(&items))
        }
        Value::Set(items) => {
            let items = items.borrow();
            Ok(in_.clone_values(&items))
        }
        Value::Iterator(iterator) => iterator_values(&iterator, span, in_),
        Value::Map(entries) => entries
            .borrow()
            .iter()
            .map(|(key, value)| {
                Ok(Value::Tuple(vec![
                    in_.clone_value(key),
                    in_.clone_value(value),
                ]))
            })
            .collect(),
        other => Err(in_.runtime_error(
            span,
            format!("expected iterable value, got {}", other.render()),
        )),
    }
}

pub(crate) fn values_equal(left: &Value, right: &Value) -> bool {
    match (left, right) {
        (Value::Unit, Value::Unit) => true,
        (Value::Bool(lhs), Value::Bool(rhs)) => lhs == rhs,
        (Value::Int(lhs), Value::Int(rhs)) => lhs == rhs,
        (Value::Float(lhs), Value::Float(rhs)) => lhs == rhs,
        (Value::String(lhs), Value::String(rhs)) => lhs == rhs,
        (Value::Rune(lhs), Value::Rune(rhs)) => lhs == rhs,
        (Value::Tuple(lhs), Value::Tuple(rhs)) => {
            lhs.len() == rhs.len() && lhs.iter().zip(rhs).all(|(lhs, rhs)| values_equal(lhs, rhs))
        }
        (Value::List(lhs), Value::List(rhs)) => {
            let lhs = lhs.borrow();
            let rhs = rhs.borrow();
            lhs.len() == rhs.len()
                && lhs
                    .iter()
                    .zip(rhs.iter())
                    .all(|(lhs, rhs)| values_equal(lhs, rhs))
        }
        (Value::Set(lhs), Value::Set(rhs)) => {
            let lhs = lhs.borrow();
            let rhs = rhs.borrow();
            lhs.len() == rhs.len()
                && lhs
                    .iter()
                    .zip(rhs.iter())
                    .all(|(lhs, rhs)| values_equal(lhs, rhs))
        }
        (Value::Map(lhs), Value::Map(rhs)) => {
            let lhs = lhs.borrow();
            let rhs = rhs.borrow();
            lhs.len() == rhs.len()
                && lhs
                    .iter()
                    .zip(rhs.iter())
                    .all(|((lk, lv), (rk, rv))| values_equal(lk, rk) && values_equal(lv, rv))
        }
        (Value::Record(lhs), Value::Record(rhs)) => {
            let lhs = lhs.borrow();
            let rhs = rhs.borrow();
            lhs.len() == rhs.len()
                && lhs
                    .iter()
                    .zip(rhs.iter())
                    .all(|((ln, lv), (rn, rv))| ln == rn && values_equal(lv, rv))
        }
        (Value::Aggregate(lhs), Value::Aggregate(rhs)) => {
            let lhs = lhs.borrow();
            let rhs = rhs.borrow();
            lhs.type_name == rhs.type_name
                && lhs.case_name == rhs.case_name
                && lhs.fields.len() == rhs.fields.len()
                && lhs
                    .fields
                    .iter()
                    .zip(rhs.fields.iter())
                    .all(|(lv, rv)| values_equal(lv, rv))
        }
        _ => false,
    }
}

fn numeric_binary_or_method(
    left: Value,
    right: Value,
    span: Option<Span>,
    method: &str,
    int_op: impl FnOnce(i64, i64) -> i64,
    float_op: impl FnOnce(f64, f64) -> f64,
    in_: &mut Interpreter<'_>,
) -> Result<Value, Diagnostic> {
    match (left.clone(), right.clone()) {
        (Value::Int(lhs), Value::Int(rhs)) => Ok(Value::Int(int_op(lhs, rhs))),
        (lhs, rhs)
            if matches!(lhs, Value::Float(_) | Value::Int(_))
                && matches!(rhs, Value::Float(_) | Value::Int(_)) =>
        {
            Ok(Value::Float(float_op(
                lhs.as_number(in_, span, "numeric binary operator")?,
                rhs.as_number(in_, span, "numeric binary operator")?,
            )))
        }
        _ => in_.invoke_method(left, method, vec![right], span),
    }
}

fn compare_binary(
    left: Value,
    right: Value,
    span: Option<Span>,
    op: impl FnOnce(f64, f64) -> bool,
    in_: &Interpreter<'_>,
) -> Result<Value, Diagnostic> {
    Ok(Value::Bool(op(
        left.as_number(in_, span, "comparison")?,
        right.as_number(in_, span, "comparison")?,
    )))
}

fn default_span() -> Span {
    let pos = LineColumn::new(1, 1);
    Span::new(0, 0, pos, pos)
}

fn decode_string_literal(raw: &str) -> String {
    let (is_raw, quoted) = raw
        .strip_prefix("raw")
        .map_or((false, raw), |quoted| (true, quoted));
    let body = if quoted.starts_with("\"\"\"") && quoted.ends_with("\"\"\"") && quoted.len() >= 6 {
        &quoted[3..quoted.len() - 3]
    } else {
        quoted
            .strip_prefix('"')
            .and_then(|value| value.strip_suffix('"'))
            .unwrap_or(quoted)
    };
    if is_raw {
        return body.to_string();
    }
    decode_string_contents(body)
}

fn decode_string_contents(body: &str) -> String {
    let mut out = String::new();
    let mut chars = body.chars();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some('t') => out.push('\t'),
            Some('\\') => out.push('\\'),
            Some('"') => out.push('"'),
            Some('0') => out.push('\0'),
            Some('$') => out.push('$'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

#[derive(Debug, Clone, Copy, Default)]
struct PrintfSpec {
    verb: char,
    left_align: bool,
    force_sign: bool,
    zero_pad: bool,
    alternate: bool,
    width: Option<usize>,
    precision: Option<usize>,
}

fn format_printf(format: &str, args: &[Value]) -> Result<String, String> {
    let chars: Vec<char> = format.chars().collect();
    let mut out = String::new();
    let mut index = 0usize;
    let mut arg_index = 0usize;

    while index < chars.len() {
        if chars[index] != '%' {
            out.push(chars[index]);
            index += 1;
            continue;
        }
        if index + 1 < chars.len() && chars[index + 1] == '%' {
            out.push('%');
            index += 2;
            continue;
        }

        let (spec, next_index) = parse_printf_spec(&chars, index + 1)?;
        let Some(value) = args.get(arg_index) else {
            return Err("printf is missing an argument".to_string());
        };
        arg_index += 1;
        out.push_str(&format_printf_value(value, spec));
        index = next_index;
    }

    if arg_index < args.len() {
        for value in &args[arg_index..] {
            out.push(' ');
            out.push_str(&value.render());
        }
    }

    Ok(out)
}

fn parse_printf_spec(chars: &[char], mut index: usize) -> Result<(PrintfSpec, usize), String> {
    let mut spec = PrintfSpec::default();

    while index < chars.len() {
        match chars[index] {
            '-' => spec.left_align = true,
            '+' => spec.force_sign = true,
            '0' => spec.zero_pad = true,
            '#' => spec.alternate = true,
            ' ' => {}
            _ => break,
        }
        index += 1;
    }

    let width_start = index;
    while index < chars.len() && chars[index].is_ascii_digit() {
        index += 1;
    }
    if index > width_start {
        spec.width = chars[width_start..index]
            .iter()
            .collect::<String>()
            .parse::<usize>()
            .ok();
    }

    if index < chars.len() && chars[index] == '.' {
        index += 1;
        let precision_start = index;
        while index < chars.len() && chars[index].is_ascii_digit() {
            index += 1;
        }
        let precision_digits: String = chars[precision_start..index].iter().collect();
        spec.precision = Some(if precision_digits.is_empty() {
            0
        } else {
            precision_digits
                .parse::<usize>()
                .map_err(|_| "invalid printf precision".to_string())?
        });
    }

    let Some(&verb) = chars.get(index) else {
        return Err("dangling '%' in printf format".to_string());
    };
    spec.verb = verb;
    Ok((spec, index + 1))
}

fn format_printf_value(value: &Value, spec: PrintfSpec) -> String {
    let rendered = match spec.verb {
        's' => {
            let mut text = match value {
                Value::String(text) => text.clone(),
                other => other.render(),
            };
            if let Some(precision) = spec.precision {
                text = text.chars().take(precision).collect();
            }
            text
        }
        'q' => match value {
            Value::String(text) => format!("{text:?}"),
            other => format!("{:?}", other.render()),
        },
        'd' => render_int_like(value, 10, false, spec.force_sign, spec.alternate),
        'x' => render_int_like(value, 16, false, spec.force_sign, spec.alternate),
        'X' => render_int_like(value, 16, true, spec.force_sign, spec.alternate),
        'o' => render_int_like(value, 8, false, spec.force_sign, spec.alternate),
        'b' => render_int_like(value, 2, false, spec.force_sign, spec.alternate),
        'f' => render_float_like(value, FloatVerb::Fixed, spec.precision, spec.force_sign),
        'e' => render_float_like(value, FloatVerb::LowerExp, spec.precision, spec.force_sign),
        'E' => render_float_like(value, FloatVerb::UpperExp, spec.precision, spec.force_sign),
        'g' | 'G' => render_float_like(value, FloatVerb::General, spec.precision, spec.force_sign),
        't' => match value {
            Value::Bool(flag) => flag.to_string(),
            other => other.render(),
        },
        'v' => value.render(),
        _ => {
            let mut text = String::from("%");
            text.push(spec.verb);
            text.push_str(&value.render());
            text
        }
    };

    apply_printf_width(rendered, spec)
}

fn render_int_like(
    value: &Value,
    radix: u32,
    uppercase: bool,
    force_sign: bool,
    alternate: bool,
) -> String {
    let Some(number) = value_as_i64(value) else {
        return value.render();
    };

    let abs = number.unsigned_abs();
    let mut digits = match radix {
        2 => format!("{abs:b}"),
        8 => format!("{abs:o}"),
        16 if uppercase => format!("{abs:X}"),
        16 => format!("{abs:x}"),
        _ => abs.to_string(),
    };
    if alternate {
        let prefix = match radix {
            2 => "0b",
            8 => "0o",
            16 if uppercase => "0X",
            16 => "0x",
            _ => "",
        };
        digits = format!("{prefix}{digits}");
    }
    if number < 0 {
        format!("-{digits}")
    } else if force_sign {
        format!("+{digits}")
    } else {
        digits
    }
}

#[derive(Debug, Clone, Copy)]
enum FloatVerb {
    Fixed,
    LowerExp,
    UpperExp,
    General,
}

fn render_float_like(
    value: &Value,
    verb: FloatVerb,
    precision: Option<usize>,
    force_sign: bool,
) -> String {
    let Some(number) = value_as_f64(value) else {
        return value.render();
    };
    let precision = precision.unwrap_or(6);
    let mut rendered = match verb {
        FloatVerb::Fixed => format!("{number:.precision$}"),
        FloatVerb::LowerExp => format!("{number:.precision$e}"),
        FloatVerb::UpperExp => format!("{number:.precision$E}"),
        FloatVerb::General => format!("{number:.precision$}"),
    };
    if force_sign && number >= 0.0 {
        rendered.insert(0, '+');
    }
    rendered
}

fn apply_printf_width(mut rendered: String, spec: PrintfSpec) -> String {
    let Some(width) = spec.width else {
        return rendered;
    };

    let rendered_len = rendered.chars().count();
    if rendered_len >= width {
        return rendered;
    }

    let pad_char = if spec.zero_pad && !spec.left_align {
        '0'
    } else {
        ' '
    };
    let pad: String = std::iter::repeat_n(pad_char, width - rendered_len).collect();

    if spec.left_align {
        rendered.push_str(&pad);
        return rendered;
    }

    if pad_char == '0' && (rendered.starts_with('-') || rendered.starts_with('+')) {
        let sign = rendered.remove(0);
        return format!("{sign}{pad}{rendered}");
    }

    format!("{pad}{rendered}")
}

fn value_as_i64(value: &Value) -> Option<i64> {
    match value {
        Value::Int(number) => Some(*number),
        Value::Float(number) => Some(*number as i64),
        _ => None,
    }
}

fn value_as_f64(value: &Value) -> Option<f64> {
    match value {
        Value::Int(number) => Some(*number as f64),
        Value::Float(number) => Some(*number),
        _ => None,
    }
}

fn is_primitive_type_name(name: &str) -> bool {
    matches!(
        name,
        "Bool" | "Float" | "Float32" | "Int" | "Int32" | "Never" | "Rune" | "Unit"
    )
}

fn render_ir_type(ty: &ir::Type) -> String {
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
                .map(render_ir_type)
                .collect::<Vec<_>>()
                .join(",")
        ),
        ir::Type::Tuple(items) => format!(
            "({})",
            items
                .iter()
                .map(render_ir_type)
                .collect::<Vec<_>>()
                .join(",")
        ),
        ir::Type::Record(fields) => format!(
            "{{{}}}",
            fields
                .iter()
                .map(|field| format!("{} {}", field.name, render_ir_type(&field.ty)))
                .collect::<Vec<_>>()
                .join(",")
        ),
        ir::Type::Function { params, ret } => format!(
            "({}) -> {}",
            params
                .iter()
                .map(render_ir_type)
                .collect::<Vec<_>>()
                .join(","),
            render_ir_type(ret)
        ),
        ir::Type::TypeParam(name) => name.clone(),
    }
}

fn push_unique_runtime_record_field(
    fields: &mut Vec<(String, Value)>,
    name: String,
    value: Value,
    span: Option<Span>,
    interpreter: &Interpreter<'_>,
) -> Result<(), Diagnostic> {
    if fields.iter().any(|(field, _)| field == &name) {
        Err(interpreter.runtime_error(
            span,
            format!(
                "shape field '{}' already exists; spread can only add fields, use ':<' to update existing fields",
                name
            ),
        ))
    } else {
        fields.push((name, value));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{SourceFile, check_program, lex, parse_program};
    use std::{
        fs,
        path::{Path, PathBuf},
    };

    fn lower_inline(src: &str) -> ir::Program {
        let file = SourceFile::new("test.lum", src);
        let lexed = lex(&file);
        assert!(lexed.diagnostics.is_empty(), "{:#?}", lexed.diagnostics);
        let parsed = parse_program(&lexed.tokens);
        assert!(parsed.diagnostics.is_empty(), "{:#?}", parsed.diagnostics);
        let program = parsed.program.expect("program");
        let checked = check_program(&program);
        assert!(checked.diagnostics.is_empty(), "{:#?}", checked.diagnostics);
        let lowered = lower_program(&program);
        assert!(lowered.diagnostics.is_empty(), "{:#?}", lowered.diagnostics);
        lowered.program.expect("lowered program")
    }

    fn repo_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../..")
            .canonicalize()
            .expect("repo root")
    }

    fn collect_lum_files(dir: &Path, out: &mut Vec<PathBuf>) {
        let entries = fs::read_dir(dir).expect("read dir");
        for entry in entries {
            let entry = entry.expect("dir entry");
            let path = entry.path();
            if path.is_dir() {
                collect_lum_files(&path, out);
            } else if path.extension().is_some_and(|ext| ext == "lum") {
                out.push(path);
            }
        }
    }

    fn should_skip_example(src: &str) -> bool {
        for line in src.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            if trimmed == "# SKIP" || trimmed.starts_with("# SKIP:") {
                return true;
            }
            if !trimmed.starts_with('#') {
                return false;
            }
        }
        false
    }

    fn parse_comment_block(src: &str, header: &str) -> Option<String> {
        let lines: Vec<&str> = src.split('\n').collect();
        let mut start = None;
        for (index, line) in lines.iter().enumerate() {
            let trimmed = line.trim();
            if trimmed == header {
                start = Some(index + 1);
                break;
            }
            if !trimmed.is_empty() && !trimmed.starts_with('#') {
                break;
            }
        }
        let start = start?;

        let mut out = Vec::new();
        for line in &lines[start..] {
            let trimmed = line.trim();
            if !trimmed.starts_with('#') {
                break;
            }
            let mut content = trimmed.trim_start_matches('#');
            if let Some(stripped) = content.strip_prefix(' ') {
                content = stripped;
            }
            out.push(content);
        }
        Some(out.join("\n"))
    }

    fn count_defined(values: &[bool]) -> usize {
        values.iter().filter(|value| **value).count()
    }

    fn normalize_example_output(value: &str) -> String {
        value.trim_end_matches('\n').to_string()
    }

    fn render_run_output(result: &PathRunResult) -> String {
        let mut actual = result.output.clone();
        if let Some(value) = &result.return_value {
            actual.push_str(value);
            actual.push('\n');
        }
        actual
    }

    fn render_located_diagnostic_for_example(diagnostic: &LocatedDiagnostic) -> String {
        format!(
            "{} at {}:{}: {}",
            diagnostic.diagnostic.code,
            diagnostic.diagnostic.span.start_pos.line,
            diagnostic.diagnostic.span.start_pos.column,
            diagnostic.diagnostic.message
        )
    }

    fn extract_primary_load_error_message(err: &str) -> Option<String> {
        let trimmed = err.trim();
        if let Some(first_line) = trimmed.lines().next() {
            if let Some((_, message)) = first_line.rsplit_once("]: ") {
                return Some(message.to_string());
            }
        }

        if !(trimmed.starts_with("parse ") || trimmed.starts_with("lex ")) {
            return None;
        }

        let Some(message_start) = trimmed.find(" error[").map(|index| index + " error[".len())
        else {
            return None;
        };
        let Some(after_code) = trimmed[message_start..]
            .find("] ")
            .map(|index| message_start + index + 2)
        else {
            return None;
        };
        let rest = &trimmed[after_code..];
        let mut end = rest.len();
        for (index, _) in rest.match_indices("; ") {
            let tail = &rest[index + 2..];
            let bytes = tail.as_bytes();
            let mut line_end = 0usize;
            while line_end < bytes.len() && bytes[line_end].is_ascii_digit() {
                line_end += 1;
            }
            if line_end == 0 || line_end >= bytes.len() || bytes[line_end] != b':' {
                continue;
            }
            let mut col_end = line_end + 1;
            while col_end < bytes.len() && bytes[col_end].is_ascii_digit() {
                col_end += 1;
            }
            if col_end == line_end + 1 {
                continue;
            }
            let remainder = &tail[col_end..];
            if remainder.starts_with(" error[") || remainder.starts_with(" warning[") {
                end = index;
                break;
            }
        }
        let first_message = rest[..end].trim();
        Some(first_message.to_string())
    }

    fn render_run_failure(path: &Path) -> String {
        match run_path(path, None) {
            Ok(result) => {
                if result.diagnostics.is_empty() {
                    return "expected example to fail, but it succeeded".to_string();
                }
                let mut seen = HashSet::new();
                let mut messages = Vec::new();
                for diagnostic in &result.diagnostics {
                    let message = render_located_diagnostic_for_example(diagnostic);
                    if seen.insert(message.clone()) {
                        messages.push(message);
                    }
                }
                messages.join("\n")
            }
            Err(err) => extract_primary_load_error_message(&err).unwrap_or(err),
        }
    }

    fn matches_failure_regex(pattern: &str, text: &str) -> bool {
        fn matches_from(pattern: &[u8], pi: usize, text: &[u8], ti: usize) -> bool {
            if pi == pattern.len() {
                return ti == text.len();
            }

            if pattern[pi..].starts_with(b"[0-9]+") {
                let mut end = ti;
                while end < text.len() && text[end].is_ascii_digit() {
                    end += 1;
                }
                for next in (ti + 1)..=end {
                    if matches_from(pattern, pi + 6, text, next) {
                        return true;
                    }
                }
                return false;
            }

            if pi + 1 < pattern.len() && pattern[pi] == b'.' && pattern[pi + 1] == b'*' {
                let next_pi = if pi + 2 < pattern.len() && pattern[pi + 2] == b'?' {
                    pi + 3
                } else {
                    pi + 2
                };
                for next in ti..=text.len() {
                    if matches_from(pattern, next_pi, text, next) {
                        return true;
                    }
                }
                return false;
            }

            if pi + 1 < pattern.len() && pattern[pi] == b'\\' {
                return ti < text.len()
                    && pattern[pi + 1] == text[ti]
                    && matches_from(pattern, pi + 2, text, ti + 1);
            }

            ti < text.len()
                && pattern[pi] == text[ti]
                && matches_from(pattern, pi + 1, text, ti + 1)
        }

        fn strip_diagnostic_prefix(text: &str) -> String {
            text.lines()
                .map(|line| {
                    line.split_once(": ")
                        .and_then(|(head, message)| head.contains(" at ").then_some(message))
                        .unwrap_or(line)
                        .to_string()
                })
                .collect::<Vec<_>>()
                .join("\n")
        }

        let stripped = strip_diagnostic_prefix(text);
        let mut variants = vec![text.to_string(), stripped.clone()];
        variants.extend(text.lines().map(str::to_string));
        variants.extend(stripped.lines().map(str::to_string));

        variants
            .into_iter()
            .any(|candidate| matches_from(pattern.as_bytes(), 0, candidate.as_bytes(), 0))
    }

    #[test]
    fn runs_class_methods_and_globals() {
        let program = lower_inline(
            r#"
            class Counter {
                hidden var count Int
            }

            impl Counter {
                new {
                    count Int
                } {
                    this.count = count
                }

                def bump(delta Int) Int {
                    this.count += delta
                    return this.count
                }
            }

            seed Int = 1

            def run() Int {
                c Counter = Counter(seed)
                return c.bump(2)
            }
            "#,
        );

        let run = run_program(&program);
        assert!(run.diagnostics.is_empty(), "{:#?}", run.diagnostics);
        assert_eq!(run.return_value.as_deref(), Some("3"));
        assert!(run.output.is_empty());
    }

    #[test]
    fn runs_range_loops_and_println() {
        let program = lower_inline(
            r#"
            def main() Unit {
                var total Int = 0
                for item <- Range(1, 4) {
                    OS.println("range", item)
                    total += item
                }
                OS.println("total", total)
            }
            "#,
        );

        let run = run_program(&program);
        assert!(run.diagnostics.is_empty(), "{:#?}", run.diagnostics);
        assert_eq!(run.output, "range 1\nrange 2\nrange 3\ntotal 6\n");
        assert_eq!(run.return_value, None);
    }

    #[test]
    fn runs_defaulted_constructor_parameters() {
        let program = lower_inline(
            r#"
            class User {
                name Str
                age Int
            }

            impl User {
                new {
                    name Str
                    age Int = 0
                } {
                    this.name = name
                    this.age = age
                }
            }

            def main() Unit {
                ada User = User("Ada")
                ben User = User { age: 12, name: "Ben" }
                OS.println(ada.name, ada.age)
                OS.println(ben.name, ben.age)
            }
            "#,
        );

        let run = run_program(&program);
        assert!(run.diagnostics.is_empty(), "{:#?}", run.diagnostics);
        assert_eq!(run.output, "Ada 0\nBen 12\n");
    }

    #[test]
    fn runs_variadic_constructor_parameters() {
        let program = lower_inline(
            r#"
            class Path {
                segments [Str]
            }

            impl Path {
                new {
                    vararg segments [Str]
                } {
                    this.segments = segments
                }

                def size() Int = this.segments.size()

                def firstOr(value Str) Str = this.segments.get(0).getOr(value)
            }

            def run() Str {
                empty Path = Path()
                path Path = Path("usr", "local", "bin")
                return empty.size() + ":" + path.size() + ":" + path.firstOr("?")
            }
            "#,
        );

        let run = run_program(&program);
        assert!(run.diagnostics.is_empty(), "{:#?}", run.diagnostics);
        assert_eq!(run.return_value.as_deref(), Some("0:3:usr"));
    }

    #[test]
    fn runs_collection_methods_and_core_operator_methods() {
        let program = lower_inline(
            r#"
            class Vec {
                hidden var items Array[Int]
            }

            impl Vec {
                new {
                    left Int
                    right Int
                } {
                    this.items = Array(left, right)
                }

                def [](index Int) Int = this.items[index]
                def +(other Vec) Vec = Vec(this[0] + other[0], this[1] + other[1])
                def -() Vec = Vec(-this[0], -this[1])
            }

            def main() Unit {
                items = List(1, 2)
                items.add(3)
                items.addAll(List(4, 5))
                OS.println(items[4])

                seen = Set(1, 2)
                seen.add(3)
                OS.println(seen.size())

                pairs = Map("a": 1)
                pairs.put("b", 2)
                OS.println(pairs.size())

                left Vec = Vec(5, 6)
                OS.println((left + Vec(1, 2))[1])
                OS.println((-left)[0])

                ints = Array.ofInt(2)
                floats = Array.ofFloat(1)
                bools = Array.ofBool(1)
                strs = Array.ofStr(1)
                runes = Array.ofRune(1)
                nul Rune = "\0".expectRuneAt(0)
                OS.println(ints[0], floats[0], bools[0], strs[0], runes[0] == nul)
            }
            "#,
        );

        let run = run_program(&program);
        assert!(run.diagnostics.is_empty(), "{:#?}", run.diagnostics);
        assert_eq!(run.output, "5\n3\n2\n8\n-5\n0 0.0 false  true\n");
        assert_eq!(run.return_value, None);
    }

    #[test]
    fn runs_list_remove_first_and_seeded_reduce() {
        let program = lower_inline(
            r#"
            def main() Int {
                values = List(1, 2, 3)
                values.removeFirst()
                return values.reduce(0, (acc, value) -> acc + value)
            }
            "#,
        );

        let run = run_program(&program);
        assert!(run.diagnostics.is_empty(), "{:#?}", run.diagnostics);
        assert_eq!(run.return_value.as_deref(), Some("5"));
        assert!(run.output.is_empty());
    }

    #[test]
    fn runs_implicit_empty_brace_constructor_when_all_fields_initialized() {
        let program = lower_inline(
            r#"
            class OrderManager {
                hidden map Map[Int, Str] = Map()
                hidden var currentTick Int = 0
                hidden queue [Str] = []
            }

            impl OrderManager {
                def current() Int = this.currentTick
                def queued() Int = this.queue.size()
                def entries() Int = this.map.size()
            }

            def main() Int {
                manager = OrderManager {}
                return manager.current() + manager.queued() + manager.entries()
            }
            "#,
        );

        let run = run_program(&program);
        assert!(run.diagnostics.is_empty(), "{:#?}", run.diagnostics);
        assert_eq!(run.return_value.as_deref(), Some("0"));
        assert!(run.output.is_empty());
    }

    #[test]
    fn runs_parenthesized_enum_constructor_with_brace_class_payload() {
        let program = lower_inline(
            r#"
            class Order {
                quantity Int
            }

            def main() Int {
                maybeOrder = Some(
                    Order {
                        quantity: 7
                    }
                )
                expect Some(order) = maybeOrder
                return order.quantity
            }
            "#,
        );

        let run = run_program(&program);
        assert!(run.diagnostics.is_empty(), "{:#?}", run.diagnostics);
        assert_eq!(run.return_value.as_deref(), Some("7"));
        assert!(run.output.is_empty());
    }

    #[test]
    fn runs_match_for_yield_and_try() {
        let program = lower_inline(
            r#"
            def countItems(items [Int]) Option[Int] {
                count = try Some(items.size())
                Some(count)
            }

            def main() Int {
                items = for item <- [1, 2, 3] yield {
                    item + 1
                }

                count = countItems(items).orPanic()

                var total Int = 0
                for item <- items {
                    total += item
                }

                OS.println("size", count)
                OS.println("total", total)

                return match count {
                    case 3 => 10
                    case _ => 20
                }
            }
            "#,
        );

        let run = run_program(&program);
        assert!(run.diagnostics.is_empty(), "{:#?}", run.diagnostics);
        assert_eq!(run.output, "size 3\ntotal 9\n");
        assert_eq!(run.return_value.as_deref(), Some("10"));
    }

    #[test]
    fn runs_for_class_destructuring_with_visible_fields() {
        let program = lower_inline(
            r#"
            class SecretUser {
                name Str
                hidden token Str
                location Str
            }

            impl SecretUser {
                new {
                    name Str
                    token Str
                    location Str
                } {
                    this.name = name
                    this.token = token
                    this.location = location
                }
            }

            def main() Unit {
                users = List(
                    SecretUser("Sergey", "secret-1", "Tampa"),
                    SecretUser("Ada", "secret-2", "London"),
                )

                for user <- users {
                    let { name as userName, location as userLocation } = user
                    OS.println("pos", userName, userLocation)
                }

                for user <- users {
                    let { location as loc, name } = user
                    OS.println("named", name, loc)
                }
            }
            "#,
        );

        let run = run_program(&program);
        assert!(run.diagnostics.is_empty(), "{:#?}", run.diagnostics);
        assert_eq!(
            run.output,
            "pos Sergey Tampa\npos Ada London\nnamed Sergey Tampa\nnamed Ada London\n"
        );
    }

    #[test]
    fn runs_option_and_result_methods() {
        let program = lower_inline(
            r#"
            def main() Unit {
                some = Some(5)
                none = None
                ok = Ok(9)
                err = Err("missing")
                OS.println("some", some.getOr(0))
                OS.println("none", none.isEmpty())
                OS.println("ok", ok.getOr(0))
                OS.println("err", err.getError())
            }
            "#,
        );

        let run = run_program(&program);
        assert!(run.diagnostics.is_empty(), "{:#?}", run.diagnostics);
        assert_eq!(run.output, "some 5\nnone true\nok 9\nerr missing\n");
    }

    #[test]
    fn runs_string_interpolation_multiline_strings_and_printf() {
        let program = lower_inline(
            r#"
            def main() Unit {
                name Str = "world"
                count Int = 6
                text Str = """
hello
$name
\n
"""
                rawSingle Str = raw"$name\n"
                rawMulti Str = raw"""$name
\n"""
                OS.println("hello $name ${count + 1} \$done")
                OS.println(text)
                OS.println(rawSingle)
                OS.println(rawMulti)
                OS.printf("fmt %d\n", 7)
                OS.stdout.printf("pair %s %d\n", "left", 9)
            }
            "#,
        );

        let run = run_program(&program);
        assert!(run.diagnostics.is_empty(), "{:#?}", run.diagnostics);
        assert_eq!(
            run.output,
            "hello world 7 $done\n\nhello\nworld\n\n\n\n$name\\n\n$name\n\\n\nfmt 7\npair left 9\n"
        );
    }

    #[test]
    fn runs_string_rune_access_methods() {
        let program = lower_inline(
            r#"
            def main() Unit {
                word Str = "apple"
                first Rune = word.expectRuneAt(0)
                expect second <- word.runeAt(1)
                missing = word.runeAt(9)

                OS.println(first)
                OS.println(second)
                OS.println(missing.isEmpty())
            }
            "#,
        );

        let run = run_program(&program);
        assert!(run.diagnostics.is_empty(), "{:#?}", run.diagnostics);
        assert_eq!(run.output, "a\np\ntrue\n");
    }

    #[test]
    fn runs_collection_from_add_and_add_all_methods() {
        let program = lower_inline(
            r#"
            def main() Unit {
                base = [1, 2]
                grown = List.from(base)
                grown.add(3)
                grown.addAll([4, 5])

                seen = Set(1, 2)
                more = Set.from(seen)
                more.add(3)
                more.addAll([2, 4, 4])

                OS.println("base", base.size(), base.get(1).getOr(0))
                OS.println("grown", grown.size(), grown.get(4).getOr(0))
                OS.println("seen", seen.size(), seen.contains(3))
                OS.println("more", more.size(), more.contains(4))
            }
            "#,
        );

        let run = run_program(&program);
        assert!(run.diagnostics.is_empty(), "{:#?}", run.diagnostics);
        assert_eq!(
            run.output,
            "base 2 2\ngrown 5 5\nseen 2 false\nmore 4 true\n"
        );
    }

    #[test]
    fn runs_named_class_destructuring_bindings() {
        let program = lower_inline(
            r#"
            class User {
                name Str
                location Str
                age Int
            }

            def main() Unit {
                user User = User { name: "Sergey", location: "Tampa", age: 37 }

                let { name, location } = user
                OS.println(name, location)

                let { name as nameAgain } = user
                OS.println(nameAgain)

                let { name as nam, location as loc } = user
                OS.println(nam, loc)

                let { name } = user
                OS.println(name)
            }
            "#,
        );

        let run = run_program(&program);
        assert!(run.diagnostics.is_empty(), "{:#?}", run.diagnostics);
        assert_eq!(run.output, "Sergey Tampa\nSergey\nSergey Tampa\nSergey\n");
    }

    #[test]
    fn runs_trailing_block_lambda_call_syntax() {
        let program = lower_inline(
            r#"
            def main() Unit {
                empty [Int] = []
                mappedEmpty = empty.map { value -> value + 5 }

                values [Int] = [1, 2]
                mapped = values.map { value -> value + 5 }

                OS.println(mappedEmpty.size())
                OS.println(mapped.get(0).getOr(0))
                OS.println(mapped.get(1).getOr(0))
                OS.println(mapped.size())
            }
            "#,
        );

        let run = run_program(&program);
        assert!(run.diagnostics.is_empty(), "{:#?}", run.diagnostics);
        assert_eq!(run.output, "0\n6\n7\n2\n");
    }

    fn collect_header_parity_failures(include_failures: bool) -> (Vec<String>, Vec<String>) {
        let root = repo_root();
        let mut files = Vec::new();
        collect_lum_files(&root.join("examples"), &mut files);
        files.sort();

        let mut failures = Vec::new();
        let mut passed = Vec::new();
        for path in files {
            let text = fs::read_to_string(&path).expect("source text");
            if should_skip_example(&text) {
                continue;
            }

            let expected_output = parse_comment_block(&text, "# EXPECT:");
            let expected_failure = parse_comment_block(&text, "# FAIL:");
            let expected_failure_regex = parse_comment_block(&text, "# FAIL_REGEX:");

            if count_defined(&[
                expected_output.is_some(),
                expected_failure.is_some(),
                expected_failure_regex.is_some(),
            ]) > 1
            {
                failures.push(format!(
                    "{}\nexample cannot declare more than one of # EXPECT, # FAIL, or # FAIL_REGEX",
                    path.strip_prefix(&root).unwrap_or(&path).display()
                ));
                continue;
            }

            if expected_output.is_none()
                && expected_failure.is_none()
                && expected_failure_regex.is_none()
            {
                continue;
            }

            let relative = path
                .strip_prefix(&root)
                .unwrap_or(&path)
                .display()
                .to_string();
            if let Some(expected) = expected_failure {
                if !include_failures {
                    continue;
                }
                let actual = render_run_failure(&path);
                if normalize_example_output(&actual) != normalize_example_output(&expected) {
                    failures.push(format!(
                        "{}\nexpected failure:\n{}\nactual failure:\n{}",
                        relative, expected, actual
                    ));
                } else {
                    passed.push(relative);
                }
                continue;
            }

            if let Some(expected_regex) = expected_failure_regex {
                if !include_failures {
                    continue;
                }
                let actual = normalize_example_output(&render_run_failure(&path));
                if !matches_failure_regex(&expected_regex, &actual) {
                    failures.push(format!(
                        "{}\nexpected failure regex:\n{}\nactual failure:\n{}",
                        relative, expected_regex, actual
                    ));
                } else {
                    passed.push(relative);
                }
                continue;
            }

            let Some(expected) = expected_output else {
                continue;
            };

            match run_path(&path, None) {
                Ok(result) => {
                    if !result.diagnostics.is_empty() {
                        let rendered = result
                            .diagnostics
                            .iter()
                            .map(render_located_diagnostic_for_example)
                            .collect::<Vec<_>>()
                            .join("\n");
                        failures.push(format!(
                            "{}\nexpected output:\n{}\nactual diagnostics:\n{}",
                            relative, expected, rendered
                        ));
                        continue;
                    }

                    let actual = render_run_output(&result);
                    if normalize_example_output(&actual) != normalize_example_output(&expected) {
                        failures.push(format!(
                            "{}\nexpected:\n{}\nactual:\n{}",
                            relative, expected, actual
                        ));
                    } else {
                        passed.push(relative);
                    }
                }
                Err(err) => failures.push(format!(
                    "{}\nexpected output:\n{}\nrun_path error:\n{}",
                    relative, expected, err
                )),
            }
        }

        (failures, passed)
    }

    #[test]
    fn run_path_matches_expected_output_headers_for_examples() {
        let (failures, passed) = collect_header_parity_failures(false);

        if failures.is_empty() {
            println!("Rust # EXPECT parity passed for {} examples:", passed.len());
            for relative in &passed {
                println!("PASS {}", relative);
            }
        }

        assert!(
            failures.is_empty(),
            "Rust # EXPECT parity failures:\n\n{}",
            failures.join("\n\n")
        );
    }

    #[test]
    fn run_path_matches_all_headers_for_examples() {
        let (failures, passed) = collect_header_parity_failures(true);

        if failures.is_empty() {
            println!("Rust example parity passed for {} examples:", passed.len());
            for relative in &passed {
                println!("PASS {}", relative);
            }
        }

        assert!(
            failures.is_empty(),
            "Rust example parity failures:\n\n{}",
            failures.join("\n\n")
        );
    }

    #[test]
    fn runs_enum_methods_and_default_case_values() {
        let program = lower_inline(
            r#"
            enum Color {
                color Str
                temperature Int

                case Black {
                    color = "xxx"
                    temperature = 1
                }
                case Red {
                    color = "xxx2"
                    temperature = 10
                }

                def isReddish() Bool = this.temperature % 5 == 0
            }

            enum OptionX[T] {
                case NoneX
                case SomeX {
                    value T
                }

                def isDefined() Bool = this != None
            }

            def main() Unit {
                black = Color.Black
                someInt = OptionX.SomeX(5)
                noneInt = OptionX.NoneX

                OS.println("reddish", black.isReddish())
                OS.println("defined", someInt.isDefined())
                OS.println("none", noneInt == OptionX.NoneX)
            }
            "#,
        );

        let run = run_program(&program);
        assert!(run.diagnostics.is_empty(), "{:#?}", run.diagnostics);
        assert_eq!(run.output, "reddish false\ndefined true\nnone true\n");
    }

    #[test]
    fn runs_enum_and_single_with_same_name() {
        let program = lower_inline(
            r#"
            enum Color {
                case Red
                case Blue

                def label() Str = match this {
                    case Color.Red => "red"
                    case Color.Blue => "blue"
                }
            }

            single Color {
            }

            impl single Color {
                def palette() Str = "palette"
            }

            def main() Unit {
                color Color = Color.Red
                OS.println(color.label())
                OS.println(Color.palette())
            }
            "#,
        );

        let run = run_program(&program);
        assert!(run.diagnostics.is_empty(), "{:#?}", run.diagnostics);
        assert_eq!(run.output, "red\npalette\n");
    }

    #[test]
    fn runs_impl_single_with_explicit_single_decl() {
        let program = lower_inline(
            r#"
            class Box {
                value Int
            }

            single Box {
            }

            impl single Box {
                def from(value Int) Box = Box { value: value }
            }

            def main() Unit {
                box = Box.from(7)
                OS.println(box.value)
            }
            "#,
        );

        let run = run_program(&program);
        assert!(run.diagnostics.is_empty(), "{:#?}", run.diagnostics);
        assert_eq!(run.output, "7\n");
    }

    #[test]
    fn runs_parse_and_list_string_helpers() {
        let program = lower_inline(
            r#"
            def main() Unit {
                values = ["btc", "usd"]
                expect Some(parsedFloat) = Float.parse("1.2")
                expect Some(parsedInt) = Int.parse("7")
                OS.println(parsedFloat + 0.8)
                OS.println(parsedInt + 1)
                OS.println(Float.parse("oops").isEmpty())
                OS.println(Int.parse("nope").isEmpty())
                OS.println(values.makeStr("-"))
                OS.println(values.nonEmpty())
            }
            "#,
        );

        let run = run_program(&program);
        assert!(run.diagnostics.is_empty(), "{:#?}", run.diagnostics);
        assert_eq!(run.output, "2.0\n8\ntrue\ntrue\nbtc-usd\ntrue\n");
    }

    #[test]
    fn runs_option_when_static_helper() {
        let program = lower_inline(
            r#"
            def main() Unit {
                someValue = Option.when(true, 7)
                noValue = Option.when(false, 7)
                OS.println(someValue.orPanic())
                OS.println(noValue.isEmpty())
            }
            "#,
        );

        let run = run_program(&program);
        assert!(run.diagnostics.is_empty(), "{:#?}", run.diagnostics);
        assert_eq!(run.output, "7\ntrue\n");
    }

    #[test]
    fn runs_match_patterns_for_shapes_classes_and_partial_enums() {
        let program = lower_inline(
            r#"
            class Amount {
                count Int
                label Str
            }

            class PairBox {
                left Int
                right Int
            }

            enum MaybeInt {
                case NoneX
                case SomeX {
                    value Int
                }
            }

            def main() Unit {
                amount Amount = Amount(42, "hello")
                pair PairBox = PairBox(5, 9)
                values [MaybeInt] = [MaybeInt.SomeX(1), MaybeInt.NoneX, MaybeInt.SomeX(3)]
                partialMapped List[Option[Int]] = values.map(value -> partial match value {
                    case SomeX(x) => x + 1
                })

                OS.println(match amount {
                    case Amount(count, label) => count + "-" + label
                })
                OS.println(match pair {
                    case PairBox(left, right) => left + right
                })
                let Some(first) = partialMapped.get(0) else return ()
                let Some(second) = partialMapped.get(1) else return ()
                OS.println(first.getOr(0))
                OS.println(second.isEmpty())
            }
            "#,
        );

        let run = run_program(&program);
        assert!(run.diagnostics.is_empty(), "{:#?}", run.diagnostics);
        assert_eq!(run.output, "42-hello\n14\n2\ntrue\n");
    }

    #[test]
    fn runs_global_shape_updates_through_synthetic_initializer() {
        let program = lower_inline(
            r#"
            class Amount {
                amount Int
                description Str
                count Int
            }

            impl Amount {
                def multiple(other Amount) Amount = Amount {
                    amount: this.amount * other.amount
                    description: this.description + " " + other.description
                    count: 0
                }
            }

            a1 = Amount(10, "description", 5)
            a2 = a1.multiple(a1)
            a3 = a2 :< { amount: 101, description: a2.description + " updated" }
            a4 = (a3 :< { amount: 102 }) :< { count: 7 }

            def main() Unit {
                OS.println(a2.amount, a2.description)
                OS.println(a3.amount, a3.description)
                OS.println(a4.amount, a4.description)
                OS.println(a4.count)
            }
            "#,
        );

        let run = run_program(&program);
        assert!(run.diagnostics.is_empty(), "{:#?}", run.diagnostics);
        assert_eq!(
            run.output,
            "100 description description\n101 description description updated\n102 description description updated\n7\n"
        );
    }

    #[test]
    fn runs_non_constant_field_initializers_before_new() {
        let program = lower_inline(
            r#"
            class Portfolio {
                assets [Str] = ["btc", "usd"]
                assetCount Int = this.assets.size()
                total Int
            }

            impl Portfolio {
                new {
                } {
                    this.total = this.assetCount + 1
                }
            }

            def main() Unit {
                portfolio = Portfolio {}
                OS.println(portfolio.assets.makeStr("-"))
                OS.println(portfolio.assetCount)
                OS.println(portfolio.total)
            }
            "#,
        );

        let run = run_program(&program);
        assert!(run.diagnostics.is_empty(), "{:#?}", run.diagnostics);
        assert_eq!(run.output, "btc-usd\n2\n3\n");
    }

    #[test]
    fn runs_nested_constructor_patterns_with_shared_case_names() {
        let program = lower_inline(
            r#"
            class Apple {
                size Int
            }

            class Amount {
                count Int
                label Str
            }

            enum MaybeApple {
                case NoneX
                case SomeX {
                    value Apple
                }
            }

            enum MaybeAmount {
                case NoneX
                case SomeX {
                    value Amount
                }
            }

            def main() Unit {
                apple = Apple(12)
                OS.println(match MaybeApple.SomeX(apple) {
                    case SomeX(Apple(size)) => "apple " + size
                    case MaybeApple.NoneX => "apple none"
                })
                amount = Amount(13, "cad")
                OS.println(match MaybeAmount.SomeX(amount) {
                    case SomeX(Amount(count, label)) => "amount " + count + " " + label
                    case MaybeAmount.NoneX => "amount none"
                })
            }
            "#,
        );

        let run = run_program(&program);
        assert!(run.diagnostics.is_empty(), "{:#?}", run.diagnostics);
        assert_eq!(run.output, "apple 12\namount 13 cad\n");
    }

    #[test]
    fn runs_if_unwrap_bindings() {
        let program = lower_inline(
            r#"
            def main() Int {
                values = [7]
                if let Some(value) = values.get(0) {
                    OS.println("binding " + value)
                } else {
                    OS.println("binding none")
                }
                return 0
            }
            "#,
        );

        let run = run_program(&program);
        assert!(run.diagnostics.is_empty(), "{:#?}", run.diagnostics);
        assert_eq!(run.output, "binding 7\n");
        assert_eq!(run.return_value.as_deref(), Some("0"));
    }

    #[test]
    fn runs_plain_let_and_irrefutable_for_body_bindings() {
        let program = lower_inline(
            r#"
            class Pair {
                left Int
                right Int
            }

            def main() Unit {
                pair Pair = Pair(5, 9)
                let Pair(item, other) = pair
                OS.println("let", item + other)

                allSome = [Some(5), Some(6)]
                for maybeItem <- allSome {
                    expect Some(loopItem) = maybeItem
                    OS.println("known", loopItem)
                }

                knownMapped = for maybeItem <- allSome yield {
                    expect Some(mappedItem) = maybeItem
                    mappedItem + 1
                }

                for result <- knownMapped {
                    OS.println("known yield", result)
                }

                pairs = List(Pair(1, 10), Pair(2, 20), Pair(3, 30))

                for pairItem <- pairs {
                    let Pair(left, right) = pairItem
                    OS.println("for", left, right)
                }

                mapped = for pairItem <- pairs yield {
                    let Pair(left, right) = pairItem
                    left + right
                }

                for result <- mapped {
                    OS.println("yield", result)
                }
            }
            "#,
        );

        let run = run_program(&program);
        assert!(run.diagnostics.is_empty(), "{:#?}", run.diagnostics);
        assert_eq!(
            run.output,
            "let 14\nknown 5\nknown 6\nknown yield 6\nknown yield 7\nfor 1 10\nfor 2 20\nfor 3 30\nyield 11\nyield 22\nyield 33\n"
        );
    }

    #[test]
    fn runs_expect_pattern_bindings() {
        let program = lower_inline(
            r#"
            def main() Unit {
                first Option[Int] = Some(5)
                expect Some(value) = first
                OS.println("expect", value)
            }
            "#,
        );

        let run = run_program(&program);
        assert!(run.diagnostics.is_empty(), "{:#?}", run.diagnostics);
        assert_eq!(run.output, "expect 5\n");
    }

    #[test]
    fn runs_assert_runtime_calls() {
        let program = lower_inline(
            r#"
            def main() Unit {
                split = "BTC-USD-5.0".split("-")
                assert(split.size() == 3)
                OS.assert(split.size() == 3, "split should have 3 parts")
                OS.println("ok")
            }
            "#,
        );

        let run = run_program(&program);
        assert!(run.diagnostics.is_empty(), "{:#?}", run.diagnostics);
        assert_eq!(run.output, "ok\n");
    }

    #[test]
    fn fails_assert_runtime_call_with_message() {
        let program = lower_inline(
            r#"
            def main() Unit {
                assert(false, "split must have 3 parts")
            }
            "#,
        );

        let run = run_program(&program);
        assert!(
            run.diagnostics
                .iter()
                .any(|diag| diag.message.contains("panic: split must have 3 parts")),
            "{:#?}",
            run.diagnostics
        );
    }

    #[test]
    fn runs_regex_string_split() {
        let program = lower_inline(
            r#"
            def main() Unit {
                split = "1234, BUY, 10, NEW".split("\s*,\s*")
                assert(split.size() == 4)
                OS.println(split[0])
                OS.println(split[1])
                OS.println(split[2])
                OS.println(split[3])
            }
            "#,
        );

        let run = run_program(&program);
        assert!(run.diagnostics.is_empty(), "{:#?}", run.diagnostics);
        assert_eq!(run.output, "1234\nBUY\n10\nNEW\n");
    }

    #[test]
    fn runs_trailing_block_lambda_with_multiline_body() {
        let program = lower_inline(
            r#"
            def main() Unit {
                items = [1, 2, 3]
                items.forEach { item ->
                    plusOne = item + 1
                    OS.println(plusOne)
                }
            }
            "#,
        );

        let run = run_program(&program);
        assert!(run.diagnostics.is_empty(), "{:#?}", run.diagnostics);
        assert_eq!(run.output, "2\n3\n4\n");
    }

    #[test]
    fn runs_anonymous_interface_methods() {
        let program = lower_inline(
            r#"
            interface Reader {
                def read() Str
            }

            interface Closer {
                def close() Unit
            }

            def main() Unit {
                handler = Reader with Closer {
                    def read() Str = "x"
                    def close() Unit = OS.println("closed")
                }

                soloReader = Reader {
                    def read() Str = "solo"
                }

                OS.println(handler.read())
                handler.close()
                OS.println(soloReader.read())
            }
            "#,
        );

        let run = run_program(&program);
        assert!(run.diagnostics.is_empty(), "{:#?}", run.diagnostics);
        assert_eq!(run.output, "x\nclosed\nsolo\n");
    }

    #[test]
    fn runs_interface_default_methods_and_overrides() {
        let program = lower_inline(
            r#"
            interface Hopper {
                def hop() Str = "hop"
            }

            interface FirstChoice {
                def choose() Str = "first"
            }

            interface SecondChoice {
                def choose() Str = "second"
            }

            class Rabbit with Hopper {}

            class PreferFirst with FirstChoice, SecondChoice {}

            def main() Unit {
                rabbit = Rabbit {}
                prefer = PreferFirst {}
                OS.println(rabbit.hop())
                OS.println(prefer.choose())
            }
            "#,
        );

        let run = run_program(&program);
        assert!(run.diagnostics.is_empty(), "{:#?}", run.diagnostics);
        assert_eq!(run.output, "hop\nfirst\n");
    }

    #[test]
    fn runs_enum_case_defaults_and_short_circuit_boolean_ops() {
        let program = lower_inline(
            r#"
            enum Outcome {
                tag Str

                case Left {
                    value Str
                    tag = "left"
                }
            }

            def boom() Bool {
                OS.println("boom")
                true
            }

            def main() Unit {
                left = Outcome.Left("bad")
                if true || boom() {
                    OS.println(left.tag)
                }
            }
            "#,
        );

        let run = run_program(&program);
        assert!(run.diagnostics.is_empty(), "{:#?}", run.diagnostics);
        assert_eq!(run.output, "left\n");
    }

    #[test]
    fn runs_defer_at_callable_exit_not_block_exit() {
        let program = lower_inline(
            r#"
            def cleanup(label Str) Unit {
                OS.println(label)
            }

            def main() Unit {
                defer cleanup("outer")
                {
                    defer cleanup("inner")
                    OS.println("body")
                }
                OS.println("after block")
            }
            "#,
        );

        let run = run_program(&program);
        assert!(run.diagnostics.is_empty(), "{:#?}", run.diagnostics);
        assert_eq!(run.output, "body\nafter block\ninner\nouter\n");
    }

    #[test]
    fn runs_defer_before_return_and_freezes_return_value() {
        let program = lower_inline(
            r#"
            def compute() Int {
                var current Int = 1
                defer {
                    current := 2
                    OS.println("cleanup", current)
                }
                return current
            }

            def main() Unit {
                OS.println(compute())
            }
            "#,
        );

        let run = run_program(&program);
        assert!(run.diagnostics.is_empty(), "{:#?}", run.diagnostics);
        assert_eq!(run.output, "cleanup 2\n1\n");
    }

    #[test]
    fn runs_defer_as_lambda_bound() {
        let program = lower_inline(
            r#"
            def main() Unit {
                defer OS.println("main")

                run = () -> {
                    defer OS.println("lambda")
                    OS.println("inside")
                }

                run()
                OS.println("after")
            }
            "#,
        );

        let run = run_program(&program);
        assert!(run.diagnostics.is_empty(), "{:#?}", run.diagnostics);
        assert_eq!(run.output, "inside\nlambda\nafter\nmain\n");
    }

    #[test]
    fn run_path_executes_plain_module_imports() {
        let path = repo_root().join("examples/imports.lum");
        let run = run_path(path, None).expect("run uses");
        assert!(run.diagnostics.is_empty(), "{:#?}", run.diagnostics);
        assert_eq!(run.output, "hello, Ada\n36\n");
    }

    #[test]
    fn run_path_executes_symbol_and_single_import_forms() {
        let path = repo_root().join("examples/import_forms.lum");
        let run = run_path(path, None).expect("run use forms");
        assert!(run.diagnostics.is_empty(), "{:#?}", run.diagnostics);
        assert_eq!(run.output, "A\nA\nB\n11\n112\n110\n");
        assert_eq!(run.return_value.as_deref(), Some("0"));
    }

    #[test]
    fn run_path_executes_default_exports_across_modules() {
        let path = repo_root().join("examples/pub_imports.lum");
        let run = run_path(path, None).expect("run default exports");
        assert!(run.diagnostics.is_empty(), "{:#?}", run.diagnostics);
        assert_eq!(run.output, "hello, Ada\nhello!\n");
    }
}
