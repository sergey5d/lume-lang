use std::{
    cell::RefCell,
    collections::{HashMap, HashSet, hash_map::DefaultHasher},
    fmt, fs,
    hash::{Hash, Hasher},
    io::{BufRead, BufReader, Read, Seek, SeekFrom as IoSeekFrom},
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
    pub warnings: Vec<LocatedDiagnostic>,
    pub output: String,
    pub return_value: Option<String>,
}

pub fn run_program(program: &ir::Program) -> RunResult {
    run_program_entry(program, None)
}

pub fn run_program_entry(program: &ir::Program, requested_entry: Option<&str>) -> RunResult {
    run_program_entry_with_args(program, requested_entry, &[])
}

pub fn run_program_entry_with_args(
    program: &ir::Program,
    requested_entry: Option<&str>,
    program_args: &[String],
) -> RunResult {
    let mut interpreter = Interpreter::new(program, program_args);
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

pub fn run_program_specs(program: &ir::Program) -> RunResult {
    let mut interpreter = Interpreter::new(program, &[]);
    let diagnostics = interpreter.run_specs();
    RunResult {
        diagnostics,
        output: interpreter.output,
        return_value: None,
    }
}

pub fn run_path(
    path: impl AsRef<Path>,
    requested_entry: Option<&str>,
) -> Result<PathRunResult, String> {
    run_path_with_args(path, requested_entry, &[])
}

pub fn run_path_with_args(
    path: impl AsRef<Path>,
    requested_entry: Option<&str>,
    program_args: &[String],
) -> Result<PathRunResult, String> {
    let path = path.as_ref();
    let checked = check_path(path)?;
    if !checked.diagnostics.is_empty() {
        return Ok(PathRunResult {
            diagnostics: checked.diagnostics,
            warnings: checked.warnings,
            output: String::new(),
            return_value: None,
        });
    }
    let warnings = checked.warnings;

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
            warnings,
            output: String::new(),
            return_value: None,
        });
    }

    let lowered_program = lowered
        .program
        .expect("ir program after successful lowering");
    let run = run_program_entry_with_args(&lowered_program, requested_entry, program_args);
    Ok(PathRunResult {
        diagnostics: run
            .diagnostics
            .into_iter()
            .map(|diagnostic| LocatedDiagnostic {
                path: root_module.display_path.clone(),
                diagnostic,
            })
            .collect(),
        warnings,
        output: run.output,
        return_value: run.return_value,
    })
}

pub fn test_path(path: impl AsRef<Path>) -> Result<PathRunResult, String> {
    let path = path.as_ref();
    let checked = check_path(path)?;
    if !checked.diagnostics.is_empty() {
        return Ok(PathRunResult {
            diagnostics: checked.diagnostics,
            warnings: checked.warnings,
            output: String::new(),
            return_value: None,
        });
    }
    let warnings = checked.warnings;

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
            warnings,
            output: String::new(),
            return_value: None,
        });
    }

    let lowered_program = lowered
        .program
        .expect("ir program after successful lowering");
    let run = run_program_specs(&lowered_program);
    Ok(PathRunResult {
        diagnostics: run
            .diagnostics
            .into_iter()
            .map(|diagnostic| LocatedDiagnostic {
                path: root_module.display_path.clone(),
                diagnostic,
            })
            .collect(),
        warnings,
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
        ast::Item::TypeAlias(alias) => {
            rewrite_type_ref_for_runtime(&mut alias.target, module);
        }
        ast::Item::Type(decl) => rewrite_type_decl_for_runtime(decl, module, graph),
        ast::Item::Extension(block) => rewrite_extension_block_for_runtime(block, module, graph),
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

fn rewrite_extension_block_for_runtime(
    block: &mut ast::ExtensionBlock,
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
            for condition in &mut stmt.condition_clauses {
                match condition {
                    ast::IfConditionClause::Expr(condition) => {
                        rewrite_expr_for_runtime(condition, module, graph);
                    }
                    ast::IfConditionClause::Let(clause) => {
                        rewrite_pattern_for_runtime(&mut clause.pattern, module);
                        rewrite_expr_for_runtime(&mut clause.value, module, graph);
                    }
                }
            }
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
        ast::Pattern::Alias { inner, .. } => rewrite_pattern_for_runtime(inner, module),
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
        ast::Pattern::List { elements, .. } => {
            for element in elements {
                rewrite_pattern_for_runtime(element, module);
            }
        }
        ast::Pattern::Record { path, fields, .. } => {
            rewrite_pattern_path_for_runtime(path, module);
            for field in fields {
                rewrite_pattern_for_runtime(&mut field.pattern, module);
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
        ast::Expr::Spread { value, .. } => rewrite_expr_for_runtime(value, module, graph),
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
        ast::Expr::ContextualNew { args, .. } => {
            for arg in args {
                rewrite_expr_for_runtime(&mut arg.value, module, graph);
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
        ast::Expr::AnonymousObject {
            interfaces,
            fields,
            methods,
            ..
        } => {
            for interface in interfaces {
                rewrite_type_ref_for_runtime(interface, module);
            }
            for field in fields {
                if let Some(ty) = &mut field.ty {
                    rewrite_type_ref_for_runtime(ty, module);
                }
                if let Some(initializer) = &mut field.initializer {
                    rewrite_expr_for_runtime(initializer, module, graph);
                }
            }
            for method in methods {
                rewrite_method_for_runtime(method, module, graph);
            }
        }
        ast::Expr::Unary { expr: inner, .. } => rewrite_expr_for_runtime(inner, module, graph),
        ast::Expr::Try { value, .. } => {
            rewrite_expr_for_runtime(value, module, graph);
        }
        ast::Expr::ExtractOr {
            value, fallback, ..
        } => {
            rewrite_expr_for_runtime(value, module, graph);
            rewrite_expr_for_runtime(fallback, module, graph);
        }
        ast::Expr::Return { value, .. } => {
            if let Some(value) = value {
                rewrite_expr_for_runtime(value, module, graph);
            }
        }
        ast::Expr::Break { .. } | ast::Expr::Continue { .. } => {}
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
            condition_clauses,
            then_block,
            else_branch,
            ..
        } => {
            for clause in condition_clauses {
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
            if let Some((module_alias, member)) = name.split_once('.')
                && !member.contains('.')
                && module.imports.contains_key(module_alias)
            {
                *name = member.to_string();
            } else if let Some(path) = rewritten_imported_symbol_path(module, name) {
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
        ast::TypeRef::Union { members, .. } => {
            for member in members {
                rewrite_type_ref_for_runtime(member, module);
            }
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
    if let Some(object_name) = &symbol.object_name {
        vec![object_name.clone(), symbol.original_name.clone()]
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
    FileStream(Rc<RefCell<FileStreamValue>>),
    TextFileReader(Rc<RefCell<TextFileReaderValue>>),
    ReferenceId(ReferenceIdValue),
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
    Runtime {
        id: runtime::RuntimeTypeId,
        args: Vec<ir::Type>,
    },
    Primitive(String),
    Tuple(Vec<ir::Type>),
    Function {
        params: Vec<ir::Type>,
        ret: Box<ir::Type>,
    },
    AnonymousShape(Vec<ir::NamedType>),
    Unknown,
}

#[derive(Clone)]
pub(crate) struct ReferenceIdValue {
    target: Box<Value>,
}

pub(crate) struct FileStreamValue {
    path: String,
    file: Option<fs::File>,
}

pub(crate) struct TextFileReaderValue {
    path: String,
    reader: Option<BufReader<fs::File>>,
    position: u64,
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
            Value::FileStream(stream) => {
                format!("FileStream({})", stream.borrow().path)
            }
            Value::TextFileReader(reader) => {
                format!("TextFileReader({})", reader.borrow().path)
            }
            Value::ReferenceId(_) => "<reference>".to_string(),
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
            RuntimeTypeValue::Runtime { args, .. } => {
                if args.is_empty() {
                    "<runtime>".to_string()
                } else {
                    format!(
                        "<runtime>[{}]",
                        args.iter()
                            .map(render_ir_type)
                            .collect::<Vec<_>>()
                            .join(",")
                    )
                }
            }
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
                "fn({}) {}",
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

#[derive(Debug, Clone)]
struct SpecCandidate {
    name: String,
    kind: ast::TypeKind,
    span: Option<Span>,
}

pub(crate) struct Interpreter<'a> {
    program: &'a ir::Program,
    runtime: runtime::RuntimeProgram,
    globals: Vec<Value>,
    globals_ready: bool,
    singletons: Vec<Option<Value>>,
    program_args: Vec<String>,
    output: String,
}

impl<'a> Interpreter<'a> {
    fn new(program: &'a ir::Program, program_args: &[String]) -> Self {
        let runtime = runtime::RuntimeProgram::from_ir(program);
        let singleton_count = runtime.types.len();
        let mut interpreter = Self {
            program,
            runtime,
            globals: Vec::new(),
            globals_ready: false,
            singletons: vec![None; singleton_count],
            program_args: program_args.to_vec(),
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

    fn run_specs(&mut self) -> Vec<Diagnostic> {
        if let Err(diagnostic) = self.ensure_globals() {
            return vec![diagnostic];
        }
        if self
            .lookup_type_by_kind("Spec", ast::TypeKind::Interface)
            .is_none()
        {
            return vec![self.runtime_error(
                None,
                "test runner requires interface 'Spec'; import 'spec/*' or declare Spec",
            )];
        }

        let specs = self.discover_specs();
        if specs.is_empty() {
            return vec![self.runtime_error(
                None,
                "no specs found; define a class or object that implements Spec",
            )];
        }

        let mut passed = 0usize;
        let mut diagnostics = Vec::new();
        for spec in specs {
            match self.run_spec(&spec) {
                Ok(()) => {
                    passed += 1;
                    self.output.push_str("PASS ");
                }
                Err(diagnostic) => {
                    diagnostics.push(diagnostic);
                    self.output.push_str("FAIL ");
                }
            }
            self.output.push_str(&spec.name);
            self.output.push('\n');
        }
        self.output.push('\n');
        self.output.push_str(&format!(
            "{} passed, {} failed\n",
            passed,
            diagnostics.len()
        ));

        diagnostics
    }

    fn run_spec(&mut self, spec: &SpecCandidate) -> Result<(), Diagnostic> {
        let receiver = self.instantiate_spec_receiver(spec)?;
        let Some(method) = self.find_method_overload_for_kind(&spec.name, spec.kind, "it", &[])
        else {
            return Err(self.runtime_error(
                spec.span,
                format!("spec '{}' does not provide it()", spec.name),
            ));
        };
        let _ = self.call_function(method, Some(receiver), None, Vec::new(), spec.span)?;
        Ok(())
    }

    fn discover_specs(&self) -> Vec<SpecCandidate> {
        self.runtime
            .types
            .iter()
            .filter(|ty| matches!(ty.kind, ast::TypeKind::Class | ast::TypeKind::Object))
            .filter(|ty| self.aggregate_matches_named_type(&ty.name, ty.kind, "Spec"))
            .map(|ty| SpecCandidate {
                name: ty.name.clone(),
                kind: ty.kind,
                span: self
                    .program
                    .types
                    .iter()
                    .find(|ir_ty| ir_ty.name == ty.name && ir_ty.kind == ty.kind)
                    .and_then(|ir_ty| ir_ty.span),
            })
            .collect()
    }

    fn instantiate_spec_receiver(&mut self, spec: &SpecCandidate) -> Result<Value, Diagnostic> {
        match spec.kind {
            ast::TypeKind::Class => self
                .construct_named_type(&spec.name, Vec::new(), spec.span, false)?
                .ok_or_else(|| {
                    self.runtime_error(spec.span, format!("cannot construct spec '{}'", spec.name))
                }),
            ast::TypeKind::Object => {
                self.lookup_singleton(&spec.name, spec.span)?
                    .ok_or_else(|| {
                        self.runtime_error(spec.span, format!("unknown object '{}'", spec.name))
                    })
            }
            _ => Err(self.runtime_error(
                spec.span,
                format!("type '{}' cannot be used as a spec", spec.name),
            )),
        }
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

        let execution = (|| -> Result<Value, Diagnostic> {
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
                        let scrutinee =
                            self.eval_operand(&frame, &scrutinee, block.terminator.span)?;
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
                        return value
                            .map(|operand| {
                                self.eval_operand(&frame, &operand, block.terminator.span)
                            })
                            .transpose()
                            .map(|value| value.unwrap_or(Value::Unit));
                    }
                    ir::TerminatorKind::Unreachable => {
                        return Err(self.runtime_error(
                            block.terminator.span,
                            format!("entered unreachable block in '{}'", function.name),
                        ));
                    }
                }
            }
        })();

        let returned = self.finish_callable_exit(&mut frame, execution, span)?;
        if function.name == "new" {
            if let (Some(Value::Aggregate(receiver)), Value::Aggregate(result)) =
                (frame.locals.first().cloned(), returned.clone())
            {
                let result = result.borrow();
                let mut receiver = receiver.borrow_mut();
                if receiver.type_name == result.type_name && receiver.case_name == result.case_name
                {
                    receiver.fields = result.fields.clone();
                    receiver.field_names = result.field_names.clone();
                    return Ok(Value::Unit);
                }
            }
        }
        Ok(self.coerce_value_to_type(returned, &function.return_ty))
    }

    fn finish_callable_exit(
        &mut self,
        frame: &mut Frame,
        execution: Result<Value, Diagnostic>,
        span: Option<Span>,
    ) -> Result<Value, Diagnostic> {
        let mut cleanup_failures = Vec::new();
        while let Some(deferred) = frame.defers.pop() {
            if let Err(diagnostic) = self.call_function(
                deferred.function,
                None,
                Some(deferred.captures.clone()),
                Vec::new(),
                span,
            ) {
                cleanup_failures.push(diagnostic);
            }
        }

        match execution {
            Err(mut primary) => {
                for cleanup_failure in cleanup_failures {
                    Self::attach_cleanup_failure(&mut primary, cleanup_failure);
                }
                Err(primary)
            }
            Ok(value) => {
                let mut cleanup_failures = cleanup_failures.into_iter();
                let Some(mut primary) = cleanup_failures.next() else {
                    return Ok(value);
                };
                primary
                    .notes
                    .push("error occurred while running deferred cleanup".to_string());
                for cleanup_failure in cleanup_failures {
                    Self::attach_cleanup_failure(&mut primary, cleanup_failure);
                }
                Err(primary)
            }
        }
    }

    fn attach_cleanup_failure(primary: &mut Diagnostic, cleanup_failure: Diagnostic) {
        primary.notes.push(format!(
            "deferred cleanup also failed at {}:{}: {}",
            cleanup_failure.span.start_pos.line,
            cleanup_failure.span.start_pos.column,
            cleanup_failure.message
        ));
        primary.notes.extend(
            cleanup_failure
                .notes
                .into_iter()
                .map(|note| format!("deferred cleanup note: {note}")),
        );
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
            ir::Type::Named { name, args } if name == "Vector" && args.len() == 1 => args.first(),
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
            ir::RValue::AnonymousObject {
                fields, methods, ..
            } => {
                let mut values = fields
                    .iter()
                    .map(|field| {
                        Ok((
                            field.name.clone(),
                            self.eval_operand_ref(frame, &field.value, span)?,
                        ))
                    })
                    .collect::<Result<Vec<_>, Diagnostic>>()?;
                for method in methods {
                    values.push((
                        method.name.clone(),
                        Value::Closure(Rc::new(ClosureValue {
                            function: method.function,
                            captures: method
                                .captures
                                .iter()
                                .map(|capture| self.eval_operand_ref(frame, capture, span))
                                .collect::<Result<Vec<_>, _>>()?,
                        })),
                    ));
                }
                Ok(Value::Record(Rc::new(RefCell::new(values))))
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
                let base_ty = frame
                    .and_then(|frame| self.operand_declared_type(frame.function, base))
                    .cloned();
                let base = self.eval_operand_ref(frame, base, span)?;
                let mut index = self.eval_operand_ref(frame, index, span)?;
                if let Some(ir::Type::Named { name, args }) = base_ty
                    && name == "Map"
                    && args.len() == 2
                {
                    index = self.coerce_value_to_type(index, &args[0]);
                }
                self.index_value(base, index, span)
            }
            ir::RValue::Cast { operand, ty } => {
                let value = self.eval_operand_ref(frame, operand, span)?;
                Ok(self.coerce_value_to_type(value, ty))
            }
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
            ir::Constant::OptionNone => self.option_none(),
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
            ir::Type::Named { name, args }
                if name == "Vector" || name == "LinkedList" || name == "Array" =>
            {
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
                .map(|id| RuntimeTypeValue::Runtime {
                    id,
                    args: Vec::new(),
                })
                .unwrap_or_else(|| RuntimeTypeValue::Primitive("Str".to_string())),
            ir::Type::Named { name, .. } if is_primitive_type_name(name) => {
                RuntimeTypeValue::Primitive(name.clone())
            }
            ir::Type::Named { name, args } => self
                .runtime
                .type_id_by_name_any_kind(name)
                .map(|id| RuntimeTypeValue::Runtime {
                    id,
                    args: args.clone(),
                })
                .unwrap_or_else(|| RuntimeTypeValue::Primitive(name.clone())),
            ir::Type::Union(members) => RuntimeTypeValue::Primitive(
                members
                    .iter()
                    .map(render_ir_type)
                    .collect::<Vec<_>>()
                    .join(" | "),
            ),
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
            Value::ReferenceId(_) => {
                self.runtime_type_value_for_ir_type(&ir::Type::named("ReferenceId"))
            }
            Value::FileStream(_) => {
                self.runtime_type_value_for_ir_type(&ir::Type::named("FileStream"))
            }
            Value::TextFileReader(_) => {
                self.runtime_type_value_for_ir_type(&ir::Type::named("TextFileReader"))
            }
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
                .map(|id| RuntimeTypeValue::Runtime {
                    id,
                    args: Vec::new(),
                })
                .unwrap_or(RuntimeTypeValue::Unknown),
        }
    }

    fn runtime_type_id_for_value(&self, value: &Value) -> Option<runtime::RuntimeTypeId> {
        match value {
            Value::Bool(_) => self
                .runtime
                .type_id_by_name_kind("Bool", crate::ast::TypeKind::Class),
            Value::Int(_) => self
                .runtime
                .type_id_by_name_kind("Int", crate::ast::TypeKind::Class),
            Value::Float(_) => self
                .runtime
                .type_id_by_name_kind("Float", crate::ast::TypeKind::Class),
            Value::Rune(_) => self
                .runtime
                .type_id_by_name_kind("Rune", crate::ast::TypeKind::Class),
            Value::List(_) => self
                .runtime
                .type_id_by_name_kind("Vector", crate::ast::TypeKind::Class),
            Value::Set(_) => self
                .runtime
                .type_id_by_name_kind("Set", crate::ast::TypeKind::Class),
            Value::Map(_) => self
                .runtime
                .type_id_by_name_kind("Map", crate::ast::TypeKind::Class),
            Value::ReferenceId(_) => self
                .runtime
                .type_id_by_name_kind("ReferenceId", crate::ast::TypeKind::Class),
            Value::FileStream(_) | Value::TextFileReader(_) => None,
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
            "asObject" => self.runtime_type_cast_value(runtime_type, "Object", method, args, span),
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
            "isPrivate" => {
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
            RuntimeTypeValue::Runtime { id, .. } => {
                self.runtime.type_by_id(*id).map(|ty| ty.name.clone())
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
            RuntimeTypeValue::Runtime { id, .. } => self
                .runtime
                .type_by_id(*id)
                .map(|ty| match ty.kind {
                    crate::ast::TypeKind::Annotation => "Annotation",
                    crate::ast::TypeKind::Class => "Class",
                    crate::ast::TypeKind::Record => "Shape",
                    crate::ast::TypeKind::Object => "Object",
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
        let RuntimeTypeValue::Runtime { id: type_id, .. } = runtime_type else {
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
        let RuntimeTypeValue::Runtime { id: type_id, .. } = runtime_type else {
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
        let RuntimeTypeValue::Runtime { id: type_id, .. } = runtime_type else {
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
        let RuntimeTypeValue::Runtime { id: type_id, .. } = runtime_type else {
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
        let RuntimeTypeValue::Runtime { id: type_id, .. } = runtime_type else {
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
        let RuntimeTypeValue::Runtime { id: type_id, .. } = runtime_type else {
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
                let base_ty = frame
                    .and_then(|frame| self.operand_declared_type(frame.function, base))
                    .cloned();
                let base = self.eval_operand_ref(frame, base, span)?;
                let mut index = self.eval_operand_ref(frame, index, span)?;
                if let Some(ir::Type::Named { name, args }) = base_ty
                    && name == "Map"
                    && args.len() == 2
                {
                    index = self.coerce_value_to_type(index, &args[0]);
                }
                if let Value::Map(entries) = &base {
                    self.ensure_observable_value(&index, span, "map lookup key")?;
                    let entries = entries.borrow().clone();
                    let current = self
                        .map_key_index(&entries, &index, span)?
                        .and_then(|position| entries.get(position))
                        .map(|(_, value)| value.clone());
                    return current.ok_or_else(|| {
                        self.runtime_error(
                            span,
                            format!(
                                "map compound assignment requires existing key {}",
                                index.render()
                            ),
                        )
                    });
                }
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
                let collection_ty = self.operand_declared_type(frame.function, base).cloned();
                let base = self.eval_operand(frame, base, span)?;
                let mut index = self.eval_operand(frame, index, span)?;
                let mut value = value;
                if let Some(ir::Type::Named { name, args }) = collection_ty {
                    if name == "Map" && args.len() == 2 {
                        index = self.coerce_value_to_type(index, &args[0]);
                        value = self.coerce_value_to_type(value, &args[1]);
                    } else if matches!(name.as_str(), "Array" | "Vector") && args.len() == 1 {
                        value = self.coerce_value_to_type(value, &args[0]);
                    }
                }
                self.set_index(base, index, value, span)
            }
        }
    }

    fn operand_declared_type(
        &self,
        function: ir::FunctionId,
        operand: &ir::Operand,
    ) -> Option<&ir::Type> {
        let place = match operand {
            ir::Operand::Copy(place) | ir::Operand::Move(place) => place.as_ref(),
            ir::Operand::Const(_) => return None,
        };
        match place {
            ir::Place::Local(id) => self
                .program
                .function(function)
                .and_then(|function| function.locals.get(id.0))
                .map(|local| &local.ty),
            ir::Place::Global(id) => self.program.globals.get(id.0).map(|global| &global.ty),
            ir::Place::Field { .. } | ir::Place::Index { .. } => None,
        }
    }

    fn coerce_collection_method_args(
        &self,
        receiver_ty: Option<&ir::Type>,
        method: &str,
        mut args: Vec<Value>,
    ) -> Vec<Value> {
        let Some(ir::Type::Named { name, args: types }) = receiver_ty else {
            return args;
        };

        let mut coerce = |index: usize, ty: &ir::Type| {
            if let Some(value) = args.get_mut(index) {
                *value = self.coerce_value_to_type(value.clone(), ty);
            }
        };

        match (name.as_str(), method) {
            ("Vector" | "Array" | "LinkedList", "append" | "add" | "contains")
                if types.len() == 1 =>
            {
                coerce(0, &types[0]);
            }
            ("Vector" | "Array" | "LinkedList", "setAt" | "insertAt") if types.len() == 1 => {
                coerce(1, &types[0]);
            }
            ("Set", "add" | "contains") if types.len() == 1 => {
                coerce(0, &types[0]);
            }
            ("Map", "put") if types.len() == 2 => {
                coerce(0, &types[0]);
                coerce(1, &types[1]);
            }
            ("Map", "get" | "contains") if types.len() == 2 => {
                coerce(0, &types[0]);
            }
            _ => {}
        }

        args
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
                let receiver_ty = frame
                    .and_then(|frame| self.operand_declared_type(frame.function, receiver))
                    .cloned();
                let args = self.coerce_collection_method_args(receiver_ty.as_ref(), method, args);
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
        if path[0] == "Math" && path.len() == 2 {
            return self.invoke_math_method(&path[1], args, span);
        }
        if path[0] == "File" && path.len() == 2 {
            return self.invoke_file_method(&path[1], args, span);
        }
        if path[0] == "Json" && path.len() == 2 {
            return self.invoke_json_method(&path[1], args, span);
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

        if path[0] == "Vector" && path.len() == 2 && path[1] == "from" {
            if args.len() != 1 {
                return Err(self.runtime_error(
                    span,
                    format!("Vector.from expects 1 argument, got {}", args.len()),
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
            return Ok(Value::set(self.unique_values(values, span)?));
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

        if type_name == "SeekFrom"
            && matches!(member.as_str(), "Start" | "Current" | "End")
            && args.is_empty()
        {
            return Ok(Some(self.builtin_enum_variant(
                "SeekFrom",
                member,
                &[],
                Vec::new(),
            )));
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
                self.construct_named_type(
                    "IntRange",
                    vec![Value::Int(start), Value::Int(end), Value::Int(step)],
                    span,
                    false,
                )?
            }
            "Vector" | "LinkedList" | "Array" => {
                Some(Value::List(Rc::new(RefCell::new(args.to_vec()))))
            }
            "Set" => Some(Value::Set(Rc::new(RefCell::new(
                self.unique_values(args.to_vec(), span)?,
            )))),
            "Map" => {
                let entries = map_entries_from_tuple_values(args.to_vec(), span, self)?;
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
                    "union variant 'None' does not accept call syntax; use 'None'",
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
                    && ty.kind != crate::ast::TypeKind::Object
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
                    "class '{}' has no implicit field constructor because non-public field '{}' has no initializer; define 'new' to initialize it",
                    ty.name, field.name
                ),
            ));
        }

        let public_fields = ty
            .fields
            .iter()
            .filter(|field| !field.hidden)
            .collect::<Vec<_>>();
        let mut aggregate = match instance {
            Value::Aggregate(value) => value.borrow_mut(),
            _ => unreachable!(),
        };
        for (name, value) in values {
            let Some(field) = public_fields.iter().find(|field| field.name == *name) else {
                continue;
            };
            aggregate.fields[field.slot.0] = self.coerce_value_to_type(value.clone(), &field.ty);
        }

        for field in public_fields {
            if !field.has_initializer && !values.iter().any(|(name, _)| *name == field.name) {
                return Err(self.runtime_error(
                    span,
                    format!(
                        "class '{}' requires construction fields that match its public constructor contract",
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
                    "class '{}' has no implicit positional constructor because non-public field '{}' has no initializer; define 'new' to initialize it",
                    ty.name, field.name
                ),
            ));
        }

        let public_fields = ty
            .fields
            .iter()
            .filter(|field| !field.hidden)
            .collect::<Vec<_>>();
        if values.len() > public_fields.len()
            || public_fields[values.len()..]
                .iter()
                .any(|field| !field.has_initializer)
        {
            return Err(self.runtime_error(
                span,
                format!(
                    "class '{}' positional construction must match public field order and may omit only trailing defaulted fields",
                    ty.name
                ),
            ));
        }

        let mut aggregate = match instance {
            Value::Aggregate(value) => value.borrow_mut(),
            _ => unreachable!(),
        };
        for (value, field) in values.iter().zip(public_fields.iter()) {
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
                let values = fields
                    .iter()
                    .map(|field| {
                        Ok((
                            field.name.clone(),
                            self.eval_operand_ref(frame, &field.value, span)?,
                        ))
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                self.construct_named_type(
                    name,
                    vec![Value::Record(Rc::new(RefCell::new(values)))],
                    span,
                    true,
                )?
                .ok_or_else(|| self.runtime_error(span, format!("cannot construct type '{name}'")))
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
        let explicit_names = parts
            .iter()
            .filter_map(|part| match part {
                ir::RecordSpreadPart::Field(field) => Some(field.name.as_str()),
                ir::RecordSpreadPart::Spread { .. } => None,
            })
            .collect::<HashSet<_>>();
        let mut out = Vec::new();
        for part in parts {
            match part {
                ir::RecordSpreadPart::Spread {
                    value: operand,
                    override_existing,
                } => {
                    let value = self.eval_operand_ref(frame, operand, span)?;
                    for (name, value) in self.record_spread_runtime_fields(value, span)? {
                        if explicit_names.contains(name.as_str()) {
                            if !out.iter().any(|(existing, _)| existing == &name) {
                                out.push((name, value));
                            }
                        } else if *override_existing {
                            upsert_runtime_record_field(&mut out, name, value);
                        } else {
                            push_unique_runtime_record_field(&mut out, name, value, span, self)?;
                        }
                    }
                }
                ir::RecordSpreadPart::Field(field) => {
                    let value = self.eval_operand_ref(frame, &field.value, span)?;
                    upsert_runtime_record_field(&mut out, field.name.clone(), value);
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
            return Err(self.runtime_error(span, format!("unknown union variant '{}'", case_name)));
        }
        if matches.len() > 1 {
            return Err(self.runtime_error(
                span,
                format!("union variant '{}' is ambiguous in this runtime", case_name),
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
                    "union variant '{display_name}' does not accept call syntax; use '{display_name}'"
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
                    "union variant '{}.{}' expects {}..{} arguments, got {}",
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
            ir::Intrinsic::ProgramArgs => {
                if !args.is_empty() {
                    return Err(self.runtime_error(span, "OS.args expects no arguments"));
                }
                Ok(Value::list(
                    self.program_args
                        .iter()
                        .cloned()
                        .map(Value::String)
                        .collect(),
                ))
            }
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
            ir::Intrinsic::Identity => {
                if args.len() != 1 {
                    return Err(self.runtime_error(span, "identity expects 1 argument"));
                }
                Ok(args.into_iter().next().expect("identity arg"))
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
            ir::Intrinsic::ListExtend => {
                if args.len() != 2 {
                    return Err(self.runtime_error(span, "ListExtend expects 2 arguments"));
                }
                self.list_extend(args[0].clone(), args[1].clone(), span)
            }
            ir::Intrinsic::ListLen => {
                if args.len() != 1 {
                    return Err(self.runtime_error(span, "ListLen expects 1 argument"));
                }
                match &args[0] {
                    Value::List(items) => Ok(Value::Int(items.borrow().len() as i64)),
                    _ => Err(self.runtime_error(span, "ListLen expects an Vector receiver")),
                }
            }
            ir::Intrinsic::ListGet => {
                if args.len() != 2 {
                    return Err(self.runtime_error(span, "ListGet expects 2 arguments"));
                }
                let index = args[1].as_int(self, span, "vector pattern index")?;
                match &args[0] {
                    Value::List(items) => {
                        let items = items.borrow();
                        let Some(value) = items.get(index as usize) else {
                            return Err(self.runtime_error(
                                span,
                                format!("vector pattern index {} out of bounds", index),
                            ));
                        };
                        Ok(self.clone_value(value))
                    }
                    _ => Err(self.runtime_error(span, "ListGet expects an Vector receiver")),
                }
            }
            ir::Intrinsic::ListSlice => {
                if args.len() != 2 {
                    return Err(self.runtime_error(span, "ListSlice expects 2 arguments"));
                }
                let start = args[1].as_int(self, span, "vector pattern slice start")?;
                match &args[0] {
                    Value::List(items) => {
                        let items = items.borrow();
                        let start = start.max(0) as usize;
                        let slice = items
                            .iter()
                            .skip(start)
                            .map(|value| self.clone_value(value))
                            .collect();
                        Ok(Value::list(slice))
                    }
                    _ => Err(self.runtime_error(span, "ListSlice expects an Vector receiver")),
                }
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
            ir::Intrinsic::UnsafeExtractSuccessValue => {
                if args.len() != 1 {
                    return Err(
                        self.runtime_error(span, "UnsafeExtractSuccessValue expects 1 argument")
                    );
                }
                if matches!(
                    &args[0],
                    Value::Aggregate(variant)
                        if matches!(
                            variant.borrow().case_name.as_deref(),
                            Some("Some" | "Ok" | "Right")
                        )
                ) {
                    return Ok(pattern_field_value(&args[0], "value").unwrap_or(Value::Unit));
                }

                let expected = match &args[0] {
                    Value::Aggregate(variant) => match variant.borrow().type_name.as_str() {
                        "Option" => "Option.Some",
                        "Result" => "Result.Ok",
                        "Either" => "Either.Right",
                        _ => "a successful lifted value",
                    },
                    _ => "a successful lifted value",
                };
                Err(self.runtime_error(span, format!("unsafe extraction expected {expected}")))
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
            ir::Intrinsic::PatternField(field_name) => {
                if args.len() != 1 {
                    return Err(self.runtime_error(span, "PatternField expects 1 argument"));
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
            _ => Err(self.runtime_error(span, format!("unknown OS method '{}'", method))),
        }
    }

    fn invoke_file_method(
        &mut self,
        method: &str,
        args: Vec<Value>,
        span: Option<Span>,
    ) -> Result<Value, Diagnostic> {
        if args.len() != 1 {
            return Err(self.runtime_error(
                span,
                format!("File.{method} expects 1 argument, got {}", args.len()),
            ));
        }
        let Value::String(path) = &args[0] else {
            return Err(self.runtime_error(span, format!("File.{method} expects a Str path")));
        };
        let path = path.clone();

        match method {
            "readBytes" => match fs::read(&path) {
                Ok(bytes) => Ok(self.result_ok(bytes_value(bytes))),
                Err(error) => Ok(self.result_err(self.file_io_error("read", &path, error))),
            },
            "readText" => match fs::read(&path) {
                Ok(bytes) => match String::from_utf8(bytes) {
                    Ok(text) => Ok(self.result_ok(Value::String(text))),
                    Err(error) => Ok(self.result_err(
                        self.file_invalid_encoding(&path, error.utf8_error().valid_up_to() as i64),
                    )),
                },
                Err(error) => Ok(self.result_err(self.file_io_error("read", &path, error))),
            },
            "open" => match fs::File::open(&path) {
                Ok(file) => Ok(self.result_ok(Value::FileStream(Rc::new(RefCell::new(
                    FileStreamValue {
                        path,
                        file: Some(file),
                    },
                ))))),
                Err(error) => Ok(self.result_err(self.file_io_error("open", &path, error))),
            },
            "openText" => match fs::File::open(&path) {
                Ok(file) => Ok(self.result_ok(Value::TextFileReader(Rc::new(RefCell::new(
                    TextFileReaderValue {
                        path,
                        reader: Some(BufReader::new(file)),
                        position: 0,
                    },
                ))))),
                Err(error) => Ok(self.result_err(self.file_io_error("open", &path, error))),
            },
            _ => Err(self.runtime_error(span, format!("unknown File method '{method}'"))),
        }
    }

    fn invoke_file_stream_method(
        &mut self,
        stream: Rc<RefCell<FileStreamValue>>,
        method: &str,
        args: Vec<Value>,
        span: Option<Span>,
    ) -> Result<Value, Diagnostic> {
        let path = stream.borrow().path.clone();
        match method {
            "path" => {
                expect_runtime_arity(self, "FileStream.path", &args, 0, span)?;
                Ok(Value::String(path))
            }
            "closed" => {
                expect_runtime_arity(self, "FileStream.closed", &args, 0, span)?;
                Ok(Value::Bool(stream.borrow().file.is_none()))
            }
            "position" => {
                expect_runtime_arity(self, "FileStream.position", &args, 0, span)?;
                let mut stream = stream.borrow_mut();
                let Some(file) = stream.file.as_mut() else {
                    return Err(self.runtime_error(
                        span,
                        format!("cannot read the position of closed file '{path}'"),
                    ));
                };
                match file.stream_position() {
                    Ok(position) => i64::try_from(position).map(Value::Int).map_err(|_| {
                        self.runtime_error(
                            span,
                            format!("position of file '{path}' exceeds the Int range"),
                        )
                    }),
                    Err(error) => Err(self.runtime_error(
                        span,
                        format!("cannot read the position of file '{path}': {error}"),
                    )),
                }
            }
            "read" => {
                expect_runtime_arity(self, "FileStream.read", &args, 1, span)?;
                let max_bytes = args[0].as_int(self, span, "FileStream.read maxBytes")?;
                if max_bytes < 0 {
                    return Ok(self.result_err(self.file_io_failure(
                        "read",
                        &path,
                        "maxBytes must be non-negative",
                    )));
                }
                let Ok(max_bytes) = usize::try_from(max_bytes) else {
                    return Ok(self.result_err(self.file_io_failure(
                        "read",
                        &path,
                        "maxBytes is too large",
                    )));
                };
                let mut stream = stream.borrow_mut();
                let Some(file) = stream.file.as_mut() else {
                    return Ok(self.result_err(self.file_closed(&path)));
                };
                let mut bytes = Vec::new();
                if bytes.try_reserve_exact(max_bytes).is_err() {
                    return Ok(self.result_err(self.file_io_failure(
                        "read",
                        &path,
                        "maxBytes is too large",
                    )));
                }
                bytes.resize(max_bytes, 0);
                match file.read(&mut bytes) {
                    Ok(read) => {
                        bytes.truncate(read);
                        Ok(self.result_ok(bytes_value(bytes)))
                    }
                    Err(error) => Ok(self.result_err(self.file_io_error("read", &path, error))),
                }
            }
            "readToEnd" => {
                expect_runtime_arity(self, "FileStream.readToEnd", &args, 0, span)?;
                let mut stream = stream.borrow_mut();
                let Some(file) = stream.file.as_mut() else {
                    return Ok(self.result_err(self.file_closed(&path)));
                };
                let mut bytes = Vec::new();
                match file.read_to_end(&mut bytes) {
                    Ok(_) => Ok(self.result_ok(bytes_value(bytes))),
                    Err(error) => Ok(self.result_err(self.file_io_error("read", &path, error))),
                }
            }
            "seek" => {
                if !matches!(args.len(), 1 | 2) {
                    return Err(self.runtime_error(
                        span,
                        format!(
                            "FileStream.seek expects 1 or 2 arguments, got {}",
                            args.len()
                        ),
                    ));
                }
                let offset = args[0].as_int(self, span, "FileStream.seek offset")?;
                let origin = if args.len() == 1 {
                    "Start"
                } else {
                    seek_from_case(&args[1]).ok_or_else(|| {
                        self.runtime_error(span, "FileStream.seek expects a SeekFrom value")
                    })?
                };
                let seek = match origin {
                    "Start" if offset >= 0 => IoSeekFrom::Start(offset as u64),
                    "Start" => {
                        return Ok(self.result_err(self.file_io_failure(
                            "seek",
                            &path,
                            "a start-relative offset cannot be negative",
                        )));
                    }
                    "Current" => IoSeekFrom::Current(offset),
                    "End" => IoSeekFrom::End(offset),
                    _ => unreachable!("validated SeekFrom case"),
                };
                let mut stream = stream.borrow_mut();
                let Some(file) = stream.file.as_mut() else {
                    return Ok(self.result_err(self.file_closed(&path)));
                };
                match file.seek(seek) {
                    Ok(position) => match i64::try_from(position) {
                        Ok(position) => Ok(self.result_ok(Value::Int(position))),
                        Err(_) => Ok(self.result_err(self.file_io_failure(
                            "seek",
                            &path,
                            "resulting position exceeds the Int range",
                        ))),
                    },
                    Err(error) => Ok(self.result_err(self.file_io_error("seek", &path, error))),
                }
            }
            "close" => {
                expect_runtime_arity(self, "FileStream.close", &args, 0, span)?;
                stream.borrow_mut().file.take();
                Ok(self.result_ok(Value::Unit))
            }
            _ => Err(self.runtime_error(span, format!("unknown FileStream method '{method}'"))),
        }
    }

    fn invoke_text_file_reader_method(
        &mut self,
        reader: Rc<RefCell<TextFileReaderValue>>,
        method: &str,
        args: Vec<Value>,
        span: Option<Span>,
    ) -> Result<Value, Diagnostic> {
        let path = reader.borrow().path.clone();
        match method {
            "path" => {
                expect_runtime_arity(self, "TextFileReader.path", &args, 0, span)?;
                Ok(Value::String(path))
            }
            "closed" => {
                expect_runtime_arity(self, "TextFileReader.closed", &args, 0, span)?;
                Ok(Value::Bool(reader.borrow().reader.is_none()))
            }
            "readLine" => {
                expect_runtime_arity(self, "TextFileReader.readLine", &args, 0, span)?;
                let mut reader = reader.borrow_mut();
                let start = reader.position;
                let Some(buffered) = reader.reader.as_mut() else {
                    return Ok(self.result_err(self.file_closed(&path)));
                };
                let mut bytes = Vec::new();
                match buffered.read_until(b'\n', &mut bytes) {
                    Ok(0) => Ok(self.result_ok(self.option_none())),
                    Ok(read) => {
                        reader.position += read as u64;
                        if bytes.last() == Some(&b'\n') {
                            bytes.pop();
                            if bytes.last() == Some(&b'\r') {
                                bytes.pop();
                            }
                        }
                        match String::from_utf8(bytes) {
                            Ok(line) => Ok(self.result_ok(self.option_some(Value::String(line)))),
                            Err(error) => Ok(self.result_err(
                                self.file_invalid_encoding(
                                    &path,
                                    i64::try_from(start)
                                        .unwrap_or(i64::MAX)
                                        .saturating_add(error.utf8_error().valid_up_to() as i64),
                                ),
                            )),
                        }
                    }
                    Err(error) => Ok(self.result_err(self.file_io_error("readLine", &path, error))),
                }
            }
            "readToEnd" => {
                expect_runtime_arity(self, "TextFileReader.readToEnd", &args, 0, span)?;
                let mut reader = reader.borrow_mut();
                let start = reader.position;
                let Some(buffered) = reader.reader.as_mut() else {
                    return Ok(self.result_err(self.file_closed(&path)));
                };
                let mut bytes = Vec::new();
                match buffered.read_to_end(&mut bytes) {
                    Ok(read) => {
                        reader.position += read as u64;
                        match String::from_utf8(bytes) {
                            Ok(text) => Ok(self.result_ok(Value::String(text))),
                            Err(error) => Ok(self.result_err(
                                self.file_invalid_encoding(
                                    &path,
                                    i64::try_from(start)
                                        .unwrap_or(i64::MAX)
                                        .saturating_add(error.utf8_error().valid_up_to() as i64),
                                ),
                            )),
                        }
                    }
                    Err(error) => {
                        Ok(self.result_err(self.file_io_error("readToEnd", &path, error)))
                    }
                }
            }
            "close" => {
                expect_runtime_arity(self, "TextFileReader.close", &args, 0, span)?;
                reader.borrow_mut().reader.take();
                Ok(self.result_ok(Value::Unit))
            }
            _ => Err(self.runtime_error(span, format!("unknown TextFileReader method '{method}'"))),
        }
    }

    fn file_closed(&self, path: &str) -> Value {
        self.builtin_enum_variant(
            "FileError",
            "Closed",
            &["path"],
            vec![Value::String(path.to_string())],
        )
    }

    fn file_invalid_encoding(&self, path: &str, offset: i64) -> Value {
        self.builtin_enum_variant(
            "FileError",
            "InvalidEncoding",
            &["path", "offset"],
            vec![Value::String(path.to_string()), Value::Int(offset)],
        )
    }

    fn file_io_failure(&self, operation: &str, path: &str, message: &str) -> Value {
        self.builtin_enum_variant(
            "FileError",
            "IoFailure",
            &["operation", "path", "message"],
            vec![
                Value::String(operation.to_string()),
                Value::String(path.to_string()),
                Value::String(message.to_string()),
            ],
        )
    }

    fn file_io_error(&self, operation: &str, path: &str, error: std::io::Error) -> Value {
        match error.kind() {
            std::io::ErrorKind::NotFound => self.builtin_enum_variant(
                "FileError",
                "NotFound",
                &["path"],
                vec![Value::String(path.to_string())],
            ),
            std::io::ErrorKind::PermissionDenied => self.builtin_enum_variant(
                "FileError",
                "AccessDenied",
                &["path"],
                vec![Value::String(path.to_string())],
            ),
            std::io::ErrorKind::InvalidData => self.file_invalid_encoding(path, -1),
            _ => self.file_io_failure(operation, path, &error.to_string()),
        }
    }

    fn invoke_json_method(
        &mut self,
        method: &str,
        args: Vec<Value>,
        span: Option<Span>,
    ) -> Result<Value, Diagnostic> {
        match method {
            "str" => {
                expect_runtime_arity(self, "Json.str", &args, 1, span)?;
                let Value::String(value) = &args[0] else {
                    return Err(self.runtime_error(span, "Json.str expects Str"));
                };
                Ok(self.json_variant("JsonString", &["value"], vec![Value::String(value.clone())]))
            }
            "int" => {
                expect_runtime_arity(self, "Json.int", &args, 1, span)?;
                let value = args[0].as_int(self, span, "Json.int value")?;
                Ok(self.json_number(value.to_string()))
            }
            "float" => {
                expect_runtime_arity(self, "Json.float", &args, 1, span)?;
                let Value::Float(value) = args[0] else {
                    return Err(self.runtime_error(span, "Json.float expects Float"));
                };
                Ok(self.json_number(render_json_float(value)))
            }
            "bool" => {
                expect_runtime_arity(self, "Json.bool", &args, 1, span)?;
                let value = args[0].as_bool(self, span, "Json.bool value")?;
                Ok(self.json_variant("JsonBool", &["value"], vec![Value::Bool(value)]))
            }
            "nil" => {
                expect_runtime_arity(self, "Json.nil", &args, 0, span)?;
                Ok(self.json_null())
            }
            "field" => {
                expect_runtime_arity(self, "Json.field", &args, 2, span)?;
                let Value::String(name) = &args[0] else {
                    return Err(self.runtime_error(span, "Json.field name must be Str"));
                };
                if !self.is_json_value(&args[1]) {
                    return Err(self.runtime_error(span, "Json.field value must be JsonValue"));
                }
                Ok(self.json_field(name.clone(), args[1].clone()))
            }
            "array" => {
                let values = self.json_variadic_values(args, false);
                if !values.iter().all(|value| self.is_json_value(value)) {
                    return Err(self.runtime_error(span, "Json.array expects JsonValue arguments"));
                }
                Ok(self.json_array(values))
            }
            "obj" => {
                let fields = self.json_variadic_values(args.clone(), true);
                if !fields.is_empty() && fields.iter().all(|value| self.is_json_field(value)) {
                    return Ok(self.json_object(fields));
                }
                if args.is_empty() {
                    return Ok(self.json_object(Vec::new()));
                }
                expect_runtime_arity(self, "Json.obj", &args, 1, span)?;
                Ok(self.json_encode(&args[0]))
            }
            "encode" => {
                expect_runtime_arity(self, "Json.encode", &args, 1, span)?;
                Ok(self.json_encode(&args[0]))
            }
            "stringify" => {
                expect_runtime_arity(self, "Json.stringify", &args, 1, span)?;
                let encoded = self.json_encode(&args[0]);
                self.render_json_value(&encoded)
                    .map(Value::String)
                    .map_err(|message| self.runtime_error(span, message))
            }
            "decode" => {
                expect_runtime_arity(self, "Json.decode", &args, 2, span)?;
                let Value::String(text) = &args[0] else {
                    return Err(self.runtime_error(span, "Json.decode text must be Str"));
                };
                let Value::RuntimeType(target) = &args[1] else {
                    return Err(self
                        .runtime_error(span, "Json.decode requires reified target type metadata"));
                };
                let parsed = match serde_json::from_str::<serde_json::Value>(text) {
                    Ok(value) => value,
                    Err(error) => return Ok(self.result_err(Value::String(error.to_string()))),
                };
                match self.decode_json_value(&parsed, target, span) {
                    Ok(value) => Ok(self.result_ok(value)),
                    Err(message) => Ok(self.result_err(Value::String(message))),
                }
            }
            _ => Err(self.runtime_error(span, format!("unknown Json method '{method}'"))),
        }
    }

    fn json_null(&self) -> Value {
        self.json_variant("JsonNull", &[], Vec::new())
    }

    fn json_number(&self, value: String) -> Value {
        self.json_variant("JsonNumber", &["value"], vec![Value::String(value)])
    }

    fn json_array(&self, values: Vec<Value>) -> Value {
        self.json_variant("JsonArray", &["values"], vec![Value::list(values)])
    }

    fn json_object(&self, fields: Vec<Value>) -> Value {
        self.json_variant("JsonObject", &["fields"], vec![Value::list(fields)])
    }

    fn json_variant(&self, case_name: &str, field_names: &[&str], fields: Vec<Value>) -> Value {
        self.builtin_enum_variant("JsonValue", case_name, field_names, fields)
    }

    fn json_field(&self, name: String, value: Value) -> Value {
        let runtime_type_id = self
            .runtime
            .type_id_by_name_kind("JsonField", crate::ast::TypeKind::Record);
        Value::Aggregate(Rc::new(RefCell::new(AggregateValue {
            runtime_type_id,
            type_name: "JsonField".to_string(),
            kind: crate::ast::TypeKind::Record,
            case_id: None,
            case_name: None,
            field_names: vec!["name".to_string(), "value".to_string()],
            fields: vec![Value::String(name), value],
        })))
    }

    fn is_json_value(&self, value: &Value) -> bool {
        matches!(value, Value::Aggregate(value) if value.borrow().type_name == "JsonValue")
    }

    fn is_json_field(&self, value: &Value) -> bool {
        matches!(value, Value::Aggregate(value) if value.borrow().type_name == "JsonField")
    }

    fn json_variadic_values(&self, args: Vec<Value>, fields: bool) -> Vec<Value> {
        if args.len() == 1
            && let Value::List(values) = &args[0]
            && values.borrow().iter().all(|value| {
                if fields {
                    self.is_json_field(value)
                } else {
                    self.is_json_value(value)
                }
            })
        {
            return values.borrow().clone();
        }
        args
    }

    fn json_encode(&self, value: &Value) -> Value {
        match value {
            value if self.is_json_value(value) => value.clone(),
            Value::Unit => self.json_null(),
            Value::Bool(value) => {
                self.json_variant("JsonBool", &["value"], vec![Value::Bool(*value)])
            }
            Value::Int(value) => self.json_number(value.to_string()),
            Value::Float(value) => self.json_number(render_json_float(*value)),
            Value::String(value) => {
                self.json_variant("JsonString", &["value"], vec![Value::String(value.clone())])
            }
            Value::Rune(value) => self.json_variant(
                "JsonString",
                &["value"],
                vec![Value::String(value.to_string())],
            ),
            Value::Tuple(values) => {
                self.json_array(values.iter().map(|value| self.json_encode(value)).collect())
            }
            Value::List(values) | Value::Set(values) => self.json_array(
                values
                    .borrow()
                    .iter()
                    .map(|value| self.json_encode(value))
                    .collect(),
            ),
            Value::Map(entries) => self.json_object(
                entries
                    .borrow()
                    .iter()
                    .map(|(key, value)| self.json_field(key.render(), self.json_encode(value)))
                    .collect(),
            ),
            Value::Record(fields) => self.json_object(
                fields
                    .borrow()
                    .iter()
                    .map(|(name, value)| self.json_field(name.clone(), self.json_encode(value)))
                    .collect(),
            ),
            Value::Aggregate(aggregate) => self.json_encode_aggregate(aggregate),
            other => self.json_variant(
                "JsonString",
                &["value"],
                vec![Value::String(other.render())],
            ),
        }
    }

    fn json_encode_aggregate(&self, aggregate: &Rc<RefCell<AggregateValue>>) -> Value {
        let aggregate = aggregate.borrow();
        if aggregate.type_name == "Option" {
            return if aggregate.case_name.as_deref() == Some("Some") {
                aggregate
                    .fields
                    .first()
                    .map(|value| self.json_encode(value))
                    .unwrap_or_else(|| self.json_null())
            } else {
                self.json_null()
            };
        }
        if aggregate.kind == crate::ast::TypeKind::Enum {
            let case_name = aggregate.case_name.clone().unwrap_or_default();
            if aggregate.fields.is_empty() {
                return self.json_variant("JsonString", &["value"], vec![Value::String(case_name)]);
            }
            let payload = self.json_object(
                aggregate
                    .field_names
                    .iter()
                    .zip(&aggregate.fields)
                    .map(|(name, value)| self.json_field(name.clone(), self.json_encode(value)))
                    .collect(),
            );
            return self.json_object(vec![self.json_field(case_name, payload)]);
        }

        let runtime_fields = aggregate
            .runtime_type_id
            .and_then(|type_id| self.runtime.type_by_id(type_id))
            .map(|ty| ty.fields.clone());
        let fields = aggregate
            .field_names
            .iter()
            .zip(&aggregate.fields)
            .enumerate()
            .filter_map(|(index, (name, value))| {
                let runtime_field = runtime_fields.as_ref().and_then(|fields| fields.get(index));
                if runtime_field.is_some_and(|field| field.hidden) {
                    return None;
                }
                if aggregate.runtime_type_id.is_some_and(|owner| {
                    runtime_field
                        .is_some_and(|field| self.json_field_is_ignored(owner, None, field.slot))
                }) {
                    return None;
                }
                let encoded_name = aggregate
                    .runtime_type_id
                    .and_then(|owner| {
                        runtime_field.and_then(|field| {
                            self.json_declared_field_name(owner, None, field.slot)
                        })
                    })
                    .unwrap_or_else(|| name.clone());
                Some(self.json_field(encoded_name, self.json_encode(value)))
            })
            .collect();
        self.json_object(fields)
    }

    fn json_field_is_ignored(
        &self,
        owner: runtime::RuntimeTypeId,
        case_id: Option<runtime::RuntimeEnumCaseId>,
        slot: runtime::RuntimeFieldSlot,
    ) -> bool {
        self.runtime_ir_field(owner, case_id, slot)
            .is_some_and(|field| {
                field
                    .annotations
                    .iter()
                    .any(|annotation| annotation_name_is(annotation, "JsonIgnore"))
            })
    }

    fn json_declared_field_name(
        &self,
        owner: runtime::RuntimeTypeId,
        case_id: Option<runtime::RuntimeEnumCaseId>,
        slot: runtime::RuntimeFieldSlot,
    ) -> Option<String> {
        let annotations = &self.runtime_ir_field(owner, case_id, slot)?.annotations;
        json_annotation_name(annotations)
    }

    fn runtime_ir_field(
        &self,
        owner: runtime::RuntimeTypeId,
        case_id: Option<runtime::RuntimeEnumCaseId>,
        slot: runtime::RuntimeFieldSlot,
    ) -> Option<&ir::Field> {
        let runtime_type = self.runtime.type_by_id(owner)?;
        let ir_type = self.program.types.get(runtime_type.ir_type_id?.0)?;
        match case_id {
            Some(case_id) => ir_type.enum_cases.get(case_id.0)?.fields.get(slot.0),
            None => ir_type.fields.get(slot.0),
        }
    }

    fn render_json_value(&self, value: &Value) -> Result<String, String> {
        let Value::Aggregate(aggregate) = value else {
            return Err("Json.stringify expected a JsonValue".to_string());
        };
        let aggregate = aggregate.borrow();
        if aggregate.type_name != "JsonValue" {
            return Err("Json.stringify expected a JsonValue".to_string());
        }
        match aggregate.case_name.as_deref() {
            Some("JsonNull") => Ok("null".to_string()),
            Some("JsonBool") => match aggregate.fields.first() {
                Some(Value::Bool(value)) => Ok(value.to_string()),
                _ => Err("invalid JsonBool value".to_string()),
            },
            Some("JsonNumber") => match aggregate.fields.first() {
                Some(Value::String(value)) => Ok(value.clone()),
                _ => Err("invalid JsonNumber value".to_string()),
            },
            Some("JsonString") => match aggregate.fields.first() {
                Some(Value::String(value)) => {
                    serde_json::to_string(value).map_err(|error| error.to_string())
                }
                _ => Err("invalid JsonString value".to_string()),
            },
            Some("JsonArray") => {
                let Some(Value::List(values)) = aggregate.fields.first() else {
                    return Err("invalid JsonArray value".to_string());
                };
                let rendered = values
                    .borrow()
                    .iter()
                    .map(|value| self.render_json_value(value))
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(format!("[{}]", rendered.join(",")))
            }
            Some("JsonObject") => {
                let Some(Value::List(fields)) = aggregate.fields.first() else {
                    return Err("invalid JsonObject value".to_string());
                };
                let rendered = fields
                    .borrow()
                    .iter()
                    .map(|field| {
                        let Value::Aggregate(field) = field else {
                            return Err("invalid JsonObject field".to_string());
                        };
                        let field = field.borrow();
                        let [Value::String(name), value] = field.fields.as_slice() else {
                            return Err("invalid JsonObject field".to_string());
                        };
                        let name =
                            serde_json::to_string(name).map_err(|error| error.to_string())?;
                        Ok(format!("{name}:{}", self.render_json_value(value)?))
                    })
                    .collect::<Result<Vec<_>, String>>()?;
                Ok(format!("{{{}}}", rendered.join(",")))
            }
            _ => Err("unknown JsonValue case".to_string()),
        }
    }

    fn decode_json_value(
        &mut self,
        value: &serde_json::Value,
        target: &RuntimeTypeValue,
        span: Option<Span>,
    ) -> Result<Value, String> {
        match target {
            RuntimeTypeValue::Primitive(name) => self.decode_json_primitive(value, name, span),
            RuntimeTypeValue::Runtime { id, args } => {
                self.decode_json_runtime(value, *id, args, span)
            }
            RuntimeTypeValue::Tuple(types) => {
                let serde_json::Value::Array(values) = value else {
                    return Err(format!("expected JSON array for {}", target.render()));
                };
                if values.len() != types.len() {
                    return Err(format!(
                        "expected {} tuple elements, got {}",
                        types.len(),
                        values.len()
                    ));
                }
                values
                    .iter()
                    .zip(types)
                    .map(|(value, ty)| self.decode_json_ir_type(value, ty, span))
                    .collect::<Result<Vec<_>, _>>()
                    .map(Value::Tuple)
            }
            RuntimeTypeValue::AnonymousShape(fields) => {
                let serde_json::Value::Object(values) = value else {
                    return Err("expected JSON object for anonymous shape".to_string());
                };
                let fields = fields
                    .iter()
                    .map(|field| {
                        let value = values.get(&field.name).ok_or_else(|| {
                            format!("missing JSON field '{}' for anonymous shape", field.name)
                        })?;
                        Ok((
                            field.name.clone(),
                            self.decode_json_ir_type(value, &field.ty, span)?,
                        ))
                    })
                    .collect::<Result<Vec<_>, String>>()?;
                Ok(Value::Record(Rc::new(RefCell::new(fields))))
            }
            RuntimeTypeValue::Function { .. } => {
                Err("cannot decode JSON as a function type".to_string())
            }
            RuntimeTypeValue::Unknown => Ok(self.json_any_value(value)),
        }
    }

    fn decode_json_ir_type(
        &mut self,
        value: &serde_json::Value,
        ty: &ir::Type,
        span: Option<Span>,
    ) -> Result<Value, String> {
        self.decode_json_value(value, &self.runtime_type_value_for_ir_type(ty), span)
    }

    fn decode_json_primitive(
        &mut self,
        value: &serde_json::Value,
        name: &str,
        span: Option<Span>,
    ) -> Result<Value, String> {
        match name {
            "Any" | "Unknown" => Ok(self.json_any_value(value)),
            "Unit" => value
                .is_null()
                .then_some(Value::Unit)
                .ok_or_else(|| "expected JSON null for Unit".to_string()),
            "Str" => Ok(Value::String(match value {
                serde_json::Value::Null => String::new(),
                serde_json::Value::String(value) => value.clone(),
                other => other.to_string(),
            })),
            "Int" | "Int64" => json_i64(value)
                .map(Value::Int)
                .ok_or_else(|| format!("expected JSON integer for {name}")),
            "Float" | "Float64" => json_f64(value)
                .map(Value::Float)
                .ok_or_else(|| format!("expected JSON number for {name}")),
            "Bool" => json_bool(value)
                .map(Value::Bool)
                .ok_or_else(|| "expected JSON boolean for Bool".to_string()),
            "Rune" => {
                let code =
                    json_i64(value).ok_or_else(|| "expected JSON integer for Rune".to_string())?;
                let code = u32::try_from(code)
                    .map_err(|_| format!("invalid Unicode scalar value {code}"))?;
                char::from_u32(code)
                    .map(Value::Rune)
                    .ok_or_else(|| format!("invalid Unicode scalar value {code}"))
            }
            other => {
                let Some(id) = self.runtime.type_id_by_name_any_kind(other) else {
                    return Err(format!("cannot decode JSON as unresolved type '{other}'"));
                };
                self.decode_json_runtime(value, id, &[], span)
            }
        }
    }

    fn decode_json_runtime(
        &mut self,
        value: &serde_json::Value,
        type_id: runtime::RuntimeTypeId,
        type_args: &[ir::Type],
        span: Option<Span>,
    ) -> Result<Value, String> {
        let runtime_type = self
            .runtime
            .type_by_id(type_id)
            .cloned()
            .ok_or_else(|| "JSON target type metadata is unavailable".to_string())?;
        match runtime_type.name.as_str() {
            "Str" | "Int" | "Int64" | "Float" | "Float64" | "Bool" | "Rune" => {
                return self.decode_json_primitive(value, &runtime_type.name, span);
            }
            "Option" => {
                if value.is_null() {
                    return Ok(self.option_none());
                }
                let inner = type_args.first().cloned().unwrap_or(ir::Type::Unknown);
                let decoded = self.decode_json_ir_type(value, &inner, span)?;
                return Ok(self.option_some(decoded));
            }
            "Vector" | "Array" | "LinkedList" => {
                let serde_json::Value::Array(values) = value else {
                    return Err(format!("expected JSON array for {}", runtime_type.name));
                };
                let inner = type_args.first().cloned().unwrap_or(ir::Type::Unknown);
                return values
                    .iter()
                    .map(|value| self.decode_json_ir_type(value, &inner, span))
                    .collect::<Result<Vec<_>, _>>()
                    .map(Value::list);
            }
            "Set" => {
                let serde_json::Value::Array(values) = value else {
                    return Err("expected JSON array for Set".to_string());
                };
                let inner = type_args.first().cloned().unwrap_or(ir::Type::Unknown);
                return values
                    .iter()
                    .map(|value| self.decode_json_ir_type(value, &inner, span))
                    .collect::<Result<Vec<_>, _>>()
                    .map(Value::set);
            }
            "Map" => {
                let serde_json::Value::Object(values) = value else {
                    return Err("expected JSON object for Map".to_string());
                };
                let key_ty = type_args.first().cloned().unwrap_or(ir::Type::Str);
                let value_ty = type_args.get(1).cloned().unwrap_or(ir::Type::Unknown);
                let entries = values
                    .iter()
                    .map(|(key, value)| {
                        let key = self.decode_json_ir_type(
                            &serde_json::Value::String(key.clone()),
                            &key_ty,
                            span,
                        )?;
                        let value = self.decode_json_ir_type(value, &value_ty, span)?;
                        Ok((key, value))
                    })
                    .collect::<Result<Vec<_>, String>>()?;
                return Ok(Value::map(entries));
            }
            "JsonValue" => return Ok(self.json_value_from_serde(value)),
            _ => {}
        }

        match runtime_type.kind {
            crate::ast::TypeKind::Class | crate::ast::TypeKind::Record => {
                self.decode_json_structured(value, &runtime_type, type_args, span)
            }
            crate::ast::TypeKind::Enum => {
                self.decode_json_union(value, &runtime_type, type_args, span)
            }
            crate::ast::TypeKind::Object => self
                .lookup_singleton(&runtime_type.name, span)
                .map_err(|diagnostic| diagnostic.message)?
                .ok_or_else(|| format!("object '{}' is unavailable", runtime_type.name)),
            _ => Err(format!(
                "cannot decode JSON as {} '{}'",
                runtime_type_kind_name(runtime_type.kind),
                runtime_type.name
            )),
        }
    }

    fn decode_json_structured(
        &mut self,
        value: &serde_json::Value,
        runtime_type: &runtime::RuntimeType,
        type_args: &[ir::Type],
        span: Option<Span>,
    ) -> Result<Value, String> {
        let serde_json::Value::Object(values) = value else {
            return Err(format!("expected JSON object for {}", runtime_type.name));
        };
        let substitutions = self.json_type_substitutions(runtime_type, type_args);
        let mut args = Vec::new();
        for field in runtime_type.fields.iter().filter(|field| !field.hidden) {
            let field_ty = substitute_runtime_type(&field.ty, &substitutions);
            let json_name = self
                .json_declared_field_name(runtime_type.id, None, field.slot)
                .unwrap_or_else(|| field.name.clone());
            let decoded = match values.get(&json_name) {
                Some(value) if !value.is_null() || is_option_ir_type(&field_ty) => {
                    self.decode_json_ir_type(value, &field_ty, span)?
                }
                _ if field.has_initializer => self.runtime_field_default_value(field),
                _ => self.json_default_for_type(&field_ty).ok_or_else(|| {
                    format!(
                        "missing JSON field '{}' for {}",
                        json_name, runtime_type.name
                    )
                })?,
            };
            args.push(decoded);
        }
        self.construct_named_type(&runtime_type.name, args, span, false)
            .map_err(|diagnostic| diagnostic.message)?
            .ok_or_else(|| format!("cannot construct {} from JSON", runtime_type.name))
    }

    fn decode_json_union(
        &mut self,
        value: &serde_json::Value,
        runtime_type: &runtime::RuntimeType,
        type_args: &[ir::Type],
        span: Option<Span>,
    ) -> Result<Value, String> {
        let substitutions = self.json_type_substitutions(runtime_type, type_args);
        let (case_name, payload) = match value {
            serde_json::Value::String(case_name) => (case_name.as_str(), None),
            serde_json::Value::Object(values) if values.len() == 1 => {
                let (case_name, payload) = values.iter().next().expect("one JSON union field");
                (case_name.as_str(), Some(payload))
            }
            _ => {
                return Err(format!(
                    "expected case name or single-case object for {}",
                    runtime_type.name
                ));
            }
        };
        let case = runtime_type
            .enum_cases
            .iter()
            .find(|case| case.name == case_name)
            .ok_or_else(|| format!("unknown {} case '{}'", runtime_type.name, case_name))?
            .clone();
        let args = if case.fields.is_empty() {
            Vec::new()
        } else {
            let Some(serde_json::Value::Object(values)) = payload else {
                return Err(format!(
                    "expected JSON object payload for case '{case_name}'"
                ));
            };
            case.fields
                .iter()
                .map(|field| {
                    let value = values.get(&field.name).ok_or_else(|| {
                        format!("missing JSON field '{}' for case '{case_name}'", field.name)
                    })?;
                    let ty = substitute_runtime_type(&field.ty, &substitutions);
                    self.decode_json_ir_type(value, &ty, span)
                })
                .collect::<Result<Vec<_>, String>>()?
        };
        self.construct_enum_case(Some(&runtime_type.name), case_name, args, span, false)
            .map_err(|diagnostic| diagnostic.message)
    }

    fn json_type_substitutions(
        &self,
        runtime_type: &runtime::RuntimeType,
        type_args: &[ir::Type],
    ) -> HashMap<String, ir::Type> {
        let Some(ir_type_id) = runtime_type.ir_type_id else {
            return HashMap::new();
        };
        self.program
            .types
            .get(ir_type_id.0)
            .map(|ty| {
                ty.type_params
                    .iter()
                    .cloned()
                    .zip(type_args.iter().cloned())
                    .collect()
            })
            .unwrap_or_default()
    }

    fn json_default_for_type(&self, ty: &ir::Type) -> Option<Value> {
        match ty {
            ir::Type::Unit
            | ir::Type::Bool
            | ir::Type::Int
            | ir::Type::Float
            | ir::Type::Str
            | ir::Type::Tuple(_)
            | ir::Type::Record(_) => Some(self.default_value_for_type(ty)),
            ir::Type::Named { name, .. }
                if matches!(
                    name.as_str(),
                    "Rune" | "Option" | "Vector" | "Array" | "LinkedList" | "Set" | "Map"
                ) =>
            {
                Some(self.default_value_for_type(ty))
            }
            _ => None,
        }
    }

    fn json_any_value(&self, value: &serde_json::Value) -> Value {
        match value {
            serde_json::Value::Null => Value::Unit,
            serde_json::Value::Bool(value) => Value::Bool(*value),
            serde_json::Value::Number(value) => value
                .as_i64()
                .map(Value::Int)
                .or_else(|| value.as_f64().map(Value::Float))
                .unwrap_or(Value::Unit),
            serde_json::Value::String(value) => Value::String(value.clone()),
            serde_json::Value::Array(values) => Value::list(
                values
                    .iter()
                    .map(|value| self.json_any_value(value))
                    .collect(),
            ),
            serde_json::Value::Object(values) => Value::map(
                values
                    .iter()
                    .map(|(key, value)| (Value::String(key.clone()), self.json_any_value(value)))
                    .collect(),
            ),
        }
    }

    fn json_value_from_serde(&self, value: &serde_json::Value) -> Value {
        match value {
            serde_json::Value::Null => self.json_null(),
            serde_json::Value::Bool(value) => {
                self.json_variant("JsonBool", &["value"], vec![Value::Bool(*value)])
            }
            serde_json::Value::Number(value) => self.json_number(value.to_string()),
            serde_json::Value::String(value) => {
                self.json_variant("JsonString", &["value"], vec![Value::String(value.clone())])
            }
            serde_json::Value::Array(values) => self.json_array(
                values
                    .iter()
                    .map(|value| self.json_value_from_serde(value))
                    .collect(),
            ),
            serde_json::Value::Object(values) => self.json_object(
                values
                    .iter()
                    .map(|(name, value)| {
                        self.json_field(name.clone(), self.json_value_from_serde(value))
                    })
                    .collect(),
            ),
        }
    }

    fn invoke_math_method(
        &mut self,
        method: &str,
        args: Vec<Value>,
        span: Option<Span>,
    ) -> Result<Value, Diagnostic> {
        if !matches!(method, "min" | "max") || args.len() != 2 {
            return Err(
                self.runtime_error(span, format!("Math.{method} expects 2 numeric arguments"))
            );
        }

        match (&args[0], &args[1]) {
            (Value::Int(left), Value::Int(right)) => Ok(Value::Int(if method == "min" {
                (*left).min(*right)
            } else {
                (*left).max(*right)
            })),
            (Value::Float(left), Value::Float(right)) => Ok(Value::Float(if method == "min" {
                left.min(*right)
            } else {
                left.max(*right)
            })),
            _ => Err(self.runtime_error(
                span,
                format!("Math.{method} requires two Int values or two Float values"),
            )),
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
            _ => Err(self.runtime_error(span, "ListAppend expects an Vector receiver")),
        }
    }

    fn list_extend(
        &mut self,
        list: Value,
        values: Value,
        span: Option<Span>,
    ) -> Result<Value, Diagnostic> {
        let values = iterable_values(values, span, self)?;
        match &list {
            Value::List(items) => {
                items.borrow_mut().extend(values);
                Ok(list)
            }
            _ => Err(self.runtime_error(span, "ListExtend expects an Vector receiver")),
        }
    }

    pub(crate) fn invoke_method(
        &mut self,
        receiver: Value,
        method: &str,
        args: Vec<Value>,
        span: Option<Span>,
    ) -> Result<Value, Diagnostic> {
        match &receiver {
            Value::FileStream(stream) => {
                return self.invoke_file_stream_method(stream.clone(), method, args, span);
            }
            Value::TextFileReader(reader) => {
                return self.invoke_text_file_reader_method(reader.clone(), method, args, span);
            }
            _ => {}
        }

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
                    if let Value::Closure(closure) = value {
                        return self.call_function(
                            closure.function,
                            Some(receiver.clone()),
                            Some(closure.captures.clone()),
                            args,
                            span,
                        );
                    }
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
        &mut self,
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
                Ok(Some(Value::Bool(
                    self.values_equal(&receiver, &args[0], span)?,
                )))
            }
            "hash" => {
                if !args.is_empty() {
                    return Err(self.runtime_error(
                        span,
                        format!("hash expects 0 arguments, got {}", args.len()),
                    ));
                }
                Ok(Some(Value::Int(self.hash_value(&receiver, span)?)))
            }
            _ => Ok(None),
        }
    }

    pub(crate) fn values_equal(
        &mut self,
        left: &Value,
        right: &Value,
        span: Option<Span>,
    ) -> Result<bool, Diagnostic> {
        if let Value::Aggregate(aggregate) = left {
            let (type_name, kind) = {
                let aggregate = aggregate.borrow();
                (aggregate.type_name.clone(), aggregate.kind)
            };
            if kind == crate::ast::TypeKind::Class {
                let Some(function) = self.find_method_overload_for_kind(
                    &type_name,
                    kind,
                    "equals",
                    &[right.clone()],
                ) else {
                    return Err(self.runtime_error(
                        span,
                        format!("class '{type_name}' does not implement equals(other)"),
                    ));
                };
                return match self.call_function(
                    function,
                    Some(left.clone()),
                    None,
                    vec![right.clone()],
                    span,
                )? {
                    Value::Bool(equal) => Ok(equal),
                    other => Err(self.runtime_error(
                        span,
                        format!("equals(other) must return Bool, got {}", other.render()),
                    )),
                };
            }
        }

        if let (Some(mut lhs), Some(mut rhs)) = (
            structural_shape_fields(left),
            structural_shape_fields(right),
        ) {
            lhs.sort_by(|left, right| left.0.cmp(&right.0));
            rhs.sort_by(|left, right| left.0.cmp(&right.0));
            if lhs.len() != rhs.len() {
                return Ok(false);
            }
            for ((left_name, left_value), (right_name, right_value)) in lhs.iter().zip(rhs.iter()) {
                if left_name != right_name || !self.values_equal(left_value, right_value, span)? {
                    return Ok(false);
                }
            }
            return Ok(true);
        }

        match (left, right) {
            (Value::ReferenceId(lhs), Value::ReferenceId(rhs)) => {
                Ok(values_identical(&lhs.target, &rhs.target))
            }
            (Value::Unit, Value::Unit) => Ok(true),
            (Value::Bool(lhs), Value::Bool(rhs)) => Ok(lhs == rhs),
            (Value::Int(lhs), Value::Int(rhs)) => Ok(lhs == rhs),
            (Value::Float(lhs), Value::Float(rhs)) => Ok(lhs == rhs),
            (Value::String(lhs), Value::String(rhs)) => Ok(lhs == rhs),
            (Value::Rune(lhs), Value::Rune(rhs)) => Ok(lhs == rhs),
            (Value::Tuple(lhs), Value::Tuple(rhs)) => {
                if lhs.len() != rhs.len() {
                    return Ok(false);
                }
                for (lhs, rhs) in lhs.iter().zip(rhs) {
                    if !self.values_equal(lhs, rhs, span)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            (Value::List(lhs), Value::List(rhs)) => {
                let lhs = lhs.borrow().clone();
                let rhs = rhs.borrow().clone();
                if lhs.len() != rhs.len() {
                    return Ok(false);
                }
                for (lhs, rhs) in lhs.iter().zip(rhs.iter()) {
                    if !self.values_equal(lhs, rhs, span)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            (Value::Set(lhs), Value::Set(rhs)) => {
                let lhs = lhs.borrow().clone();
                let rhs = rhs.borrow().clone();
                if lhs.len() != rhs.len() {
                    return Ok(false);
                }
                for (lhs, rhs) in lhs.iter().zip(rhs.iter()) {
                    if !self.values_equal(lhs, rhs, span)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            (Value::Map(lhs), Value::Map(rhs)) => {
                let lhs = lhs.borrow().clone();
                let rhs = rhs.borrow().clone();
                if lhs.len() != rhs.len() {
                    return Ok(false);
                }
                for ((left_key, left_value), (right_key, right_value)) in lhs.iter().zip(rhs.iter())
                {
                    if !self.values_equal(left_key, right_key, span)?
                        || !self.values_equal(left_value, right_value, span)?
                    {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            (Value::Aggregate(lhs), Value::Aggregate(rhs)) => {
                let (left_type, left_case, left_fields) = {
                    let lhs = lhs.borrow();
                    (
                        lhs.type_name.clone(),
                        lhs.case_name.clone(),
                        lhs.fields.clone(),
                    )
                };
                let (right_type, right_case, right_fields) = {
                    let rhs = rhs.borrow();
                    (
                        rhs.type_name.clone(),
                        rhs.case_name.clone(),
                        rhs.fields.clone(),
                    )
                };
                if left_type != right_type
                    || left_case != right_case
                    || left_fields.len() != right_fields.len()
                {
                    return Ok(false);
                }
                for (lhs, rhs) in left_fields.iter().zip(right_fields.iter()) {
                    if !self.values_equal(lhs, rhs, span)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    fn values_have_same_concrete_type(&self, left: &Value, right: &Value) -> bool {
        if let (Value::Aggregate(left_value), Value::Aggregate(right_value)) = (left, right) {
            let left_value = left_value.borrow();
            let right_value = right_value.borrow();
            if left_value.kind == ast::TypeKind::Record
                && right_value.kind == ast::TypeKind::Record
                && let (Some(left_id), Some(right_id)) =
                    (left_value.runtime_type_id, right_value.runtime_type_id)
                && let (Some(left_ty), Some(right_ty)) = (
                    self.runtime.type_by_id(left_id),
                    self.runtime.type_by_id(right_id),
                )
            {
                let mut left_fields = left_ty
                    .fields
                    .iter()
                    .map(|field| (field.name.clone(), field.ty.clone()))
                    .collect::<Vec<_>>();
                let mut right_fields = right_ty
                    .fields
                    .iter()
                    .map(|field| (field.name.clone(), field.ty.clone()))
                    .collect::<Vec<_>>();
                left_fields.sort_by(|left, right| left.0.cmp(&right.0));
                right_fields.sort_by(|left, right| left.0.cmp(&right.0));
                return left_fields == right_fields;
            }
        }

        let left_shape = structural_shape_fields(left);
        let right_shape = structural_shape_fields(right);
        match (left_shape, right_shape) {
            (Some(left_fields), Some(right_fields)) => {
                let mut left_names = left_fields
                    .into_iter()
                    .map(|(name, _)| name)
                    .collect::<Vec<_>>();
                let mut right_names = right_fields
                    .into_iter()
                    .map(|(name, _)| name)
                    .collect::<Vec<_>>();
                left_names.sort();
                right_names.sort();
                return left_names == right_names;
            }
            (Some(_), None) | (None, Some(_)) => return false,
            (None, None) => {}
        }

        match (left, right) {
            (Value::ReferenceId(_), Value::ReferenceId(_))
            | (Value::Unit, Value::Unit)
            | (Value::Bool(_), Value::Bool(_))
            | (Value::Int(_), Value::Int(_))
            | (Value::Float(_), Value::Float(_))
            | (Value::String(_), Value::String(_))
            | (Value::Rune(_), Value::Rune(_))
            | (Value::List(_), Value::List(_))
            | (Value::Set(_), Value::Set(_))
            | (Value::Map(_), Value::Map(_)) => true,
            (Value::Tuple(left_items), Value::Tuple(right_items)) => {
                left_items.len() == right_items.len()
            }
            (Value::Aggregate(left_value), Value::Aggregate(right_value)) => {
                let left_value = left_value.borrow();
                let right_value = right_value.borrow();
                left_value.type_name == right_value.type_name
                    && left_value.case_name == right_value.case_name
            }
            _ => false,
        }
    }

    pub(crate) fn value_index(
        &mut self,
        values: &[Value],
        needle: &Value,
        span: Option<Span>,
    ) -> Result<Option<usize>, Diagnostic> {
        for (index, value) in values.iter().enumerate() {
            if self.values_equal(value, needle, span)? {
                return Ok(Some(index));
            }
        }
        Ok(None)
    }

    pub(crate) fn map_key_index(
        &mut self,
        entries: &[(Value, Value)],
        needle: &Value,
        span: Option<Span>,
    ) -> Result<Option<usize>, Diagnostic> {
        for (index, (key, _)) in entries.iter().enumerate() {
            if self.values_equal(key, needle, span)? {
                return Ok(Some(index));
            }
        }
        Ok(None)
    }

    pub(crate) fn push_unique(
        &mut self,
        items: &mut Vec<Value>,
        value: Value,
        span: Option<Span>,
    ) -> Result<(), Diagnostic> {
        if self.value_index(items, &value, span)?.is_none() {
            items.push(value);
        }
        Ok(())
    }

    fn unique_values(
        &mut self,
        items: Vec<Value>,
        span: Option<Span>,
    ) -> Result<Vec<Value>, Diagnostic> {
        let mut out = Vec::new();
        for value in items {
            self.push_unique(&mut out, value, span)?;
        }
        Ok(out)
    }

    pub(crate) fn map_put_entry(
        &mut self,
        entries: &mut Vec<(Value, Value)>,
        key: Value,
        value: Value,
        span: Option<Span>,
    ) -> Result<(), Diagnostic> {
        if let Some(position) = self.map_key_index(entries, &key, span)? {
            entries[position].1 = value;
        } else {
            entries.push((key, value));
        }
        Ok(())
    }

    fn hash_value(&mut self, value: &Value, span: Option<Span>) -> Result<i64, Diagnostic> {
        let mut hasher = DefaultHasher::new();
        match value {
            Value::Unit => "Unit".hash(&mut hasher),
            Value::Bool(value) => {
                "Bool".hash(&mut hasher);
                value.hash(&mut hasher);
            }
            Value::Int(value) => {
                "Int".hash(&mut hasher);
                value.hash(&mut hasher);
            }
            Value::Float(value) => {
                "Float".hash(&mut hasher);
                let normalized = if *value == 0.0 { 0.0 } else { *value };
                normalized.to_bits().hash(&mut hasher);
            }
            Value::String(value) => {
                "Str".hash(&mut hasher);
                value.hash(&mut hasher);
            }
            Value::Rune(value) => {
                "Rune".hash(&mut hasher);
                value.hash(&mut hasher);
            }
            Value::ReferenceId(reference) => {
                "ReferenceId".hash(&mut hasher);
                reference_identity_address(&reference.target)
                    .expect("ReferenceId always retains a reference-bearing value")
                    .hash(&mut hasher);
            }
            Value::Tuple(items) => {
                "Tuple".hash(&mut hasher);
                items.len().hash(&mut hasher);
                for item in items {
                    self.hash_value(item, span)?.hash(&mut hasher);
                }
            }
            Value::Record(fields) => {
                "shape".hash(&mut hasher);
                let mut fields = fields.borrow().clone();
                fields.sort_by(|left, right| left.0.cmp(&right.0));
                for (name, value) in fields {
                    name.hash(&mut hasher);
                    self.hash_value(&value, span)?.hash(&mut hasher);
                }
            }
            Value::Aggregate(aggregate) => {
                let aggregate = aggregate.borrow();
                let type_name = aggregate.type_name.clone();
                let kind = aggregate.kind;
                let case_name = aggregate.case_name.clone();
                let field_names = aggregate.field_names.clone();
                let fields = aggregate.fields.clone();
                drop(aggregate);

                if kind == crate::ast::TypeKind::Class {
                    let Some(function) =
                        self.find_method_overload_for_kind(&type_name, kind, "hash", &[])
                    else {
                        return Err(self.runtime_error(
                            span,
                            format!("class '{type_name}' does not implement hash()"),
                        ));
                    };
                    return match self.call_function(
                        function,
                        Some(value.clone()),
                        None,
                        Vec::new(),
                        span,
                    )? {
                        Value::Int(hash) => Ok(hash),
                        other => Err(self.runtime_error(
                            span,
                            format!("hash() must return Int, got {}", other.render()),
                        )),
                    };
                }

                if kind == crate::ast::TypeKind::Record {
                    "shape".hash(&mut hasher);
                    let mut fields = field_names.into_iter().zip(fields).collect::<Vec<_>>();
                    fields.sort_by(|left, right| left.0.cmp(&right.0));
                    for (name, value) in fields {
                        name.hash(&mut hasher);
                        self.hash_value(&value, span)?.hash(&mut hasher);
                    }
                } else {
                    format!("{kind:?}").hash(&mut hasher);
                    type_name.hash(&mut hasher);
                    case_name.hash(&mut hasher);
                    for value in fields {
                        self.hash_value(&value, span)?.hash(&mut hasher);
                    }
                }
            }
            other => {
                return Err(
                    self.runtime_error(span, format!("{} does not satisfy Hashed", other.render()))
                );
            }
        }
        Ok(hasher.finish() as i64)
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
            .lookup_type_by_kind(name, crate::ast::TypeKind::Object)
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
            self.find_method_overload_for_kind(&ty.name, crate::ast::TypeKind::Object, "new", &[])
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

    fn get_member(
        &mut self,
        base: Value,
        name: &str,
        span: Option<Span>,
    ) -> Result<Value, Diagnostic> {
        if name == "runtimeType" {
            return Ok(Value::RuntimeType(self.runtime_type_value_for_value(&base)));
        }
        if name == "referenceId" {
            if reference_identity_address(&base).is_none() {
                return Err(self.runtime_error(
                    span,
                    format!("referenceId is not available on {}", base.render()),
                ));
            }
            return Ok(Value::ReferenceId(ReferenceIdValue {
                target: Box::new(base),
            }));
        }

        match base {
            Value::Aggregate(aggregate) => {
                if matches!(name, "stdout" | "stderr")
                    && aggregate.borrow().type_name == "OS"
                    && aggregate.borrow().case_name.is_none()
                {
                    return Ok(Value::Aggregate(aggregate.clone()));
                }
                let field = {
                    let aggregate_ref = aggregate.borrow();
                    self.aggregate_field_value(&aggregate_ref, name)
                };
                if let Some(field) = field {
                    return Ok(field);
                }
                self.invoke_method(Value::Aggregate(aggregate), name, Vec::new(), span)
            }
            Value::Record(fields) => {
                let field = lookup_named_field(&fields.borrow(), name);
                if let Some(field) = field {
                    return Ok(field);
                }
                self.invoke_method(Value::Record(fields), name, Vec::new(), span)
            }
            Value::Tuple(items) => tuple_member(&items, name)
                .ok_or_else(|| self.runtime_error(span, format!("tuple has no member '{}'", name))),
            other => self.invoke_method(other, name, Vec::new(), span),
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
            let entries = entries.borrow().clone();
            for (key, _) in &entries {
                self.ensure_observable_value(key, span, "map lookup key")?;
            }
            let value = self
                .map_key_index(&entries, &index, span)?
                .and_then(|position| entries.get(position))
                .map(|(_, value)| value.clone());
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
                    self.runtime_error(span, format!("array index {} out of bounds", index))
                })?;
                items.get(index).map_or_else(
                    || Err(self.runtime_error(span, format!("array index {} out of bounds", index))),
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
            let snapshot = entries.borrow().clone();
            let existing = self.map_key_index(&snapshot, &index, span)?;
            let mut entries = entries.borrow_mut();
            if let Some(position) = existing {
                entries[position].1 = value;
            } else {
                entries.push((index, value));
            }
            return Ok(());
        }
        let index = index.as_int(self, span, "index")?;
        match base {
            Value::List(items) => {
                let mut items = items.borrow_mut();
                let index = normalize_index(items.len(), index).ok_or_else(|| {
                    self.runtime_error(span, format!("array index {} out of bounds", index))
                })?;
                let Some(slot) = items.get_mut(index) else {
                    return Err(
                        self.runtime_error(span, format!("array index {} out of bounds", index))
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
                Value::Int(value) => Ok(Value::Int(value.wrapping_neg())),
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
                (Value::Int(lhs), Value::Int(rhs)) => Ok(Value::Int(lhs.wrapping_add(*rhs))),
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
                i64::wrapping_sub,
                |lhs, rhs| lhs - rhs,
                self,
            ),
            ir::BinaryOp::Mul => numeric_binary_or_method(
                left,
                right,
                span,
                "*",
                i64::wrapping_mul,
                |lhs, rhs| lhs * rhs,
                self,
            ),
            ir::BinaryOp::Div => numeric_division_or_method(left, right, span, self),
            ir::BinaryOp::Mod => numeric_remainder_or_method(left, right, span, self),
            ir::BinaryOp::Eq => {
                self.ensure_observable_value(&left, span, "equality comparison")?;
                self.ensure_observable_value(&right, span, "equality comparison")?;
                Ok(Value::Bool(self.values_equal(&left, &right, span)?))
            }
            ir::BinaryOp::NotEq => {
                self.ensure_observable_value(&left, span, "equality comparison")?;
                self.ensure_observable_value(&right, span, "equality comparison")?;
                Ok(Value::Bool(!self.values_equal(&left, &right, span)?))
            }
            ir::BinaryOp::StrictEq => {
                self.ensure_observable_value(&left, span, "strict equality comparison")?;
                self.ensure_observable_value(&right, span, "strict equality comparison")?;
                Ok(Value::Bool(
                    self.values_have_same_concrete_type(&left, &right)
                        && self.values_equal(&left, &right, span)?,
                ))
            }
            ir::BinaryOp::StrictNotEq => {
                self.ensure_observable_value(&left, span, "strict equality comparison")?;
                self.ensure_observable_value(&right, span, "strict equality comparison")?;
                Ok(Value::Bool(
                    !self.values_have_same_concrete_type(&left, &right)
                        || !self.values_equal(&left, &right, span)?,
                ))
            }
            ir::BinaryOp::Less => compare_binary(
                left,
                right,
                span,
                |lhs, rhs| lhs < rhs,
                |lhs, rhs| lhs < rhs,
                self,
            ),
            ir::BinaryOp::LessEq => compare_binary(
                left,
                right,
                span,
                |lhs, rhs| lhs <= rhs,
                |lhs, rhs| lhs <= rhs,
                self,
            ),
            ir::BinaryOp::Greater => compare_binary(
                left,
                right,
                span,
                |lhs, rhs| lhs > rhs,
                |lhs, rhs| lhs > rhs,
                self,
            ),
            ir::BinaryOp::GreaterEq => compare_binary(
                left,
                right,
                span,
                |lhs, rhs| lhs >= rhs,
                |lhs, rhs| lhs >= rhs,
                self,
            ),
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
            ir::Type::Named { name, .. } if name.contains("::") => {
                let Some((owner, case_name)) = name.split_once("::") else {
                    return false;
                };
                matches!(
                    value,
                    Value::Aggregate(aggregate)
                        if aggregate.borrow().type_name == owner
                            && aggregate.borrow().case_name.as_deref() == Some(case_name)
                )
            }
            ir::Type::Named { name, .. } => match value {
                Value::List(_) => name == "Vector" || name == "LinkedList" || name == "Array",
                Value::Set(_) => name == "Set",
                Value::Map(_) => name == "Map",
                Value::Iterator(_) => name == "Iterator",
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
                Value::Int(_) => name == "Int",
                Value::Float(_) => name == "Float",
                Value::Bool(_) => name == "Bool",
                Value::Unit => name == "Unit",
                Value::RuntimeType(runtime_type) => match name.as_str() {
                    "Type" | "Annotated" => true,
                    "ClassType" => self.runtime_type_kind_case(runtime_type) == "Class",
                    "ShapeType" => self.runtime_type_kind_case(runtime_type) == "Shape",
                    "EnumType" => self.runtime_type_kind_case(runtime_type) == "Enum",
                    "InterfaceType" => self.runtime_type_kind_case(runtime_type) == "Interface",
                    "ObjectType" => self.runtime_type_kind_case(runtime_type) == "Object",
                    "AnnotationType" => self.runtime_type_kind_case(runtime_type) == "Annotation",
                    _ => false,
                },
                Value::RuntimeField { .. } => name == "Field" || name == "Annotated",
                Value::RuntimeMethod { .. } => name == "Method" || name == "Annotated",
                Value::RuntimeParam { .. } => name == "Param",
                Value::RuntimeEnumCase { .. } => name == "EnumCase" || name == "Annotated",
                Value::ReferenceId(_) => name == "ReferenceId",
                Value::FileStream(_) => matches!(
                    name.as_str(),
                    "FileStream" | "ByteReader" | "Seekable" | "Closeable"
                ),
                Value::TextFileReader(_) => {
                    matches!(name.as_str(), "TextFileReader" | "TextReader" | "Closeable")
                }
                other => self
                    .runtime
                    .type_by_name_kind(name, crate::ast::TypeKind::Record)
                    .is_some_and(|ty| self.value_matches_runtime_shape(other, ty)),
            },
            ir::Type::Union(members) => members
                .iter()
                .any(|member| self.value_matches_type(value, member)),
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
            ir::Type::Union(members) => self.coerce_value_to_union(value, members),
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

    fn coerce_value_to_union(&self, value: Value, members: &[ir::Type]) -> Value {
        if members
            .iter()
            .any(|member| self.value_matches_union_member_without_shape_projection(&value, member))
        {
            return value;
        }

        let candidates = members
            .iter()
            .filter(|member| self.value_can_project_to_shape(&value, member))
            .collect::<Vec<_>>();
        let source_field_count = self.value_structural_field_count(&value);
        let exact = candidates
            .iter()
            .filter(|member| self.shape_field_count(member) == source_field_count)
            .copied()
            .collect::<Vec<_>>();
        let selected = if exact.len() == 1 {
            exact.first().copied()
        } else if exact.is_empty() && candidates.len() == 1 {
            candidates.first().copied()
        } else {
            None
        };
        selected
            .map(|target| self.coerce_value_to_type(value.clone(), target))
            .unwrap_or(value)
    }

    fn value_matches_union_member_without_shape_projection(
        &self,
        value: &Value,
        member: &ir::Type,
    ) -> bool {
        match member {
            ir::Type::Named { name, .. }
                if self
                    .runtime
                    .type_by_name_kind(name, crate::ast::TypeKind::Record)
                    .is_some() =>
            {
                matches!(value, Value::Aggregate(aggregate) if {
                    let aggregate = aggregate.borrow();
                    aggregate.type_name == *name
                        && aggregate.kind == crate::ast::TypeKind::Record
                })
            }
            ir::Type::Record(fields) => {
                self.value_structural_field_count(value) == Some(fields.len())
                    && self.value_matches_type(value, member)
            }
            _ => self.value_matches_type(value, member),
        }
    }

    fn value_can_project_to_shape(&self, value: &Value, target: &ir::Type) -> bool {
        match target {
            ir::Type::Named { name, .. } => self
                .runtime
                .type_by_name_kind(name, crate::ast::TypeKind::Record)
                .is_some_and(|ty| self.value_matches_runtime_shape(value, ty)),
            ir::Type::Record(_) => self.value_matches_type(value, target),
            _ => false,
        }
    }

    fn shape_field_count(&self, ty: &ir::Type) -> Option<usize> {
        match ty {
            ir::Type::Record(fields) => Some(fields.len()),
            ir::Type::Named { name, .. } => self
                .runtime
                .type_by_name_kind(name, crate::ast::TypeKind::Record)
                .map(|ty| ty.fields.iter().filter(|field| !field.hidden).count()),
            _ => None,
        }
    }

    fn value_structural_field_count(&self, value: &Value) -> Option<usize> {
        match value {
            Value::Record(fields) => Some(fields.borrow().len()),
            Value::Aggregate(aggregate) => {
                let aggregate = aggregate.borrow();
                aggregate
                    .runtime_type_id
                    .and_then(|id| self.runtime.type_by_id(id))
                    .map(|ty| ty.fields.iter().filter(|field| !field.hidden).count())
                    .or_else(|| Some(aggregate.field_names.len()))
            }
            _ => None,
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

pub(crate) fn aggregate_named_field(aggregate: &AggregateValue, name: &str) -> Option<Value> {
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

fn map_entries_from_tuple_values(
    values: Vec<Value>,
    span: Option<Span>,
    in_: &mut Interpreter<'_>,
) -> Result<Vec<(Value, Value)>, Diagnostic> {
    let mut entries = Vec::new();
    for value in values {
        match value {
            Value::Tuple(items) if items.len() == 2 => {
                in_.map_put_entry(&mut entries, items[0].clone(), items[1].clone(), span)?;
            }
            Value::Map(spread) => {
                for (key, value) in spread.borrow().iter() {
                    in_.map_put_entry(&mut entries, key.clone(), value.clone(), span)?;
                }
            }
            _ => {
                return Err(
                    in_.runtime_error(span, "Map expects tuple pair arguments or map spreads")
                );
            }
        }
    }
    Ok(entries)
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
    in_: &mut Interpreter<'_>,
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
        other => {
            let rendered = other.render();
            let iterator = in_.iter_init(other, span).map_err(|_| {
                in_.runtime_error(span, format!("expected iterable value, got {rendered}"))
            })?;
            let Value::Iterator(iterator) = iterator else {
                return Err(in_.runtime_error(
                    span,
                    format!("iterator() must return Iterator, got {}", iterator.render()),
                ));
            };
            iterator_values(&iterator, span, in_)
        }
    }
}

fn structural_shape_fields(value: &Value) -> Option<Vec<(String, Value)>> {
    match value {
        Value::Record(fields) => Some(fields.borrow().clone()),
        Value::Aggregate(aggregate) if aggregate.borrow().kind == ast::TypeKind::Record => {
            let aggregate = aggregate.borrow();
            Some(
                aggregate
                    .field_names
                    .iter()
                    .cloned()
                    .zip(aggregate.fields.iter().cloned())
                    .collect(),
            )
        }
        _ => None,
    }
}

fn bytes_value(bytes: Vec<u8>) -> Value {
    Value::list(
        bytes
            .into_iter()
            .map(|byte| Value::Int(i64::from(byte)))
            .collect(),
    )
}

fn seek_from_case(value: &Value) -> Option<&'static str> {
    let Value::Aggregate(aggregate) = value else {
        return None;
    };
    let aggregate = aggregate.borrow();
    if aggregate.type_name != "SeekFrom" {
        return None;
    }
    match aggregate.case_name.as_deref() {
        Some("Start") => Some("Start"),
        Some("Current") => Some("Current"),
        Some("End") => Some("End"),
        _ => None,
    }
}

fn expect_runtime_arity(
    interpreter: &Interpreter<'_>,
    callable: &str,
    args: &[Value],
    expected: usize,
    span: Option<Span>,
) -> Result<(), Diagnostic> {
    if args.len() == expected {
        Ok(())
    } else {
        Err(interpreter.runtime_error(
            span,
            format!(
                "{callable} expects {expected} arguments, got {}",
                args.len()
            ),
        ))
    }
}

fn values_identical(left: &Value, right: &Value) -> bool {
    match (left, right) {
        (Value::List(lhs), Value::List(rhs)) => Rc::ptr_eq(lhs, rhs),
        (Value::Set(lhs), Value::Set(rhs)) => Rc::ptr_eq(lhs, rhs),
        (Value::Map(lhs), Value::Map(rhs)) => Rc::ptr_eq(lhs, rhs),
        (Value::Record(lhs), Value::Record(rhs)) => Rc::ptr_eq(lhs, rhs),
        (Value::Aggregate(lhs), Value::Aggregate(rhs)) => Rc::ptr_eq(lhs, rhs),
        (Value::FileStream(lhs), Value::FileStream(rhs)) => Rc::ptr_eq(lhs, rhs),
        (Value::TextFileReader(lhs), Value::TextFileReader(rhs)) => Rc::ptr_eq(lhs, rhs),
        _ => false,
    }
}

fn reference_identity_address(value: &Value) -> Option<usize> {
    match value {
        Value::List(value) => Some(Rc::as_ptr(value) as usize),
        Value::Set(value) => Some(Rc::as_ptr(value) as usize),
        Value::Map(value) => Some(Rc::as_ptr(value) as usize),
        Value::Record(value) => Some(Rc::as_ptr(value) as usize),
        Value::Aggregate(value) => Some(Rc::as_ptr(value) as usize),
        Value::FileStream(value) => Some(Rc::as_ptr(value) as usize),
        Value::TextFileReader(value) => Some(Rc::as_ptr(value) as usize),
        _ => None,
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

fn numeric_division_or_method(
    left: Value,
    right: Value,
    span: Option<Span>,
    in_: &mut Interpreter<'_>,
) -> Result<Value, Diagnostic> {
    match (left.clone(), right.clone()) {
        (Value::Int(_), Value::Int(0)) => Err(in_.runtime_error(span, "integer division by zero")),
        (Value::Int(lhs), Value::Int(rhs)) => Ok(Value::Int(lhs.wrapping_div(rhs))),
        (lhs, rhs)
            if matches!(lhs, Value::Float(_) | Value::Int(_))
                && matches!(rhs, Value::Float(_) | Value::Int(_)) =>
        {
            Ok(Value::Float(
                lhs.as_number(in_, span, "numeric division")?
                    / rhs.as_number(in_, span, "numeric division")?,
            ))
        }
        _ => in_.invoke_method(left, "/", vec![right], span),
    }
}

fn numeric_remainder_or_method(
    left: Value,
    right: Value,
    span: Option<Span>,
    in_: &mut Interpreter<'_>,
) -> Result<Value, Diagnostic> {
    match (left.clone(), right.clone()) {
        (Value::Int(_), Value::Int(0)) => Err(in_.runtime_error(span, "integer remainder by zero")),
        (Value::Int(lhs), Value::Int(rhs)) => Ok(Value::Int(lhs.wrapping_rem(rhs))),
        (lhs, rhs)
            if matches!(lhs, Value::Float(_) | Value::Int(_))
                && matches!(rhs, Value::Float(_) | Value::Int(_)) =>
        {
            Ok(Value::Float(
                lhs.as_number(in_, span, "numeric remainder")?
                    % rhs.as_number(in_, span, "numeric remainder")?,
            ))
        }
        _ => in_.invoke_method(left, "%", vec![right], span),
    }
}

fn compare_binary(
    left: Value,
    right: Value,
    span: Option<Span>,
    int_op: impl FnOnce(i64, i64) -> bool,
    float_op: impl FnOnce(f64, f64) -> bool,
    in_: &mut Interpreter<'_>,
) -> Result<Value, Diagnostic> {
    match (left.clone(), right.clone()) {
        (Value::Int(lhs), Value::Int(rhs)) => Ok(Value::Bool(int_op(lhs, rhs))),
        (Value::Float(lhs), Value::Float(rhs)) => Ok(Value::Bool(float_op(lhs, rhs))),
        (Value::Int(lhs), Value::Float(rhs)) => Ok(Value::Bool(float_op(lhs as f64, rhs))),
        (Value::Float(lhs), Value::Int(rhs)) => Ok(Value::Bool(float_op(lhs, rhs as f64))),
        (Value::String(lhs), Value::String(rhs)) => {
            let comparison = lhs.cmp(&rhs) as i8 as i64;
            Ok(Value::Bool(int_op(comparison, 0)))
        }
        _ => {
            let comparison = in_
                .invoke_method(left, "compare", vec![right], span)?
                .as_int(in_, span, "Ordered.compare result")?;
            Ok(Value::Bool(int_op(comparison, 0)))
        }
    }
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

fn render_json_float(value: f64) -> String {
    if value.is_nan() {
        "NaN".to_string()
    } else if value == f64::INFINITY {
        "Infinity".to_string()
    } else if value == f64::NEG_INFINITY {
        "-Infinity".to_string()
    } else {
        serde_json::Number::from_f64(value)
            .map(|value| value.to_string())
            .unwrap_or_else(|| value.to_string())
    }
}

fn json_i64(value: &serde_json::Value) -> Option<i64> {
    match value {
        serde_json::Value::Number(value) => value.as_i64(),
        serde_json::Value::String(value) => value.parse().ok(),
        _ => None,
    }
}

fn json_f64(value: &serde_json::Value) -> Option<f64> {
    match value {
        serde_json::Value::Number(value) => value.as_f64(),
        serde_json::Value::String(value) => value.parse().ok(),
        _ => None,
    }
}

fn json_bool(value: &serde_json::Value) -> Option<bool> {
    match value {
        serde_json::Value::Bool(value) => Some(*value),
        serde_json::Value::String(value) => value.parse().ok(),
        _ => None,
    }
}

fn annotation_name_is(annotation: &ir::Annotation, expected: &str) -> bool {
    annotation
        .name
        .rsplit('.')
        .next()
        .is_some_and(|name| name == expected)
}

fn json_annotation_name(annotations: &[ir::Annotation]) -> Option<String> {
    let annotation = annotations
        .iter()
        .find(|annotation| annotation_name_is(annotation, "JsonName"))?;
    annotation.fields.iter().find_map(|field| {
        (field.name == "value")
            .then_some(&field.value)
            .and_then(|value| match value {
                ir::AnnotationValue::String(value) if !value.is_empty() => Some(value.clone()),
                _ => None,
            })
    })
}

fn is_option_ir_type(ty: &ir::Type) -> bool {
    matches!(ty, ir::Type::Named { name, .. } if name == "Option")
}

fn substitute_runtime_type(ty: &ir::Type, substitutions: &HashMap<String, ir::Type>) -> ir::Type {
    match ty {
        ir::Type::TypeParam(name) => substitutions
            .get(name)
            .cloned()
            .unwrap_or_else(|| ty.clone()),
        ir::Type::Named { name, args } => ir::Type::Named {
            name: name.clone(),
            args: args
                .iter()
                .map(|arg| substitute_runtime_type(arg, substitutions))
                .collect(),
        },
        ir::Type::Union(members) => ir::Type::Union(
            members
                .iter()
                .map(|member| substitute_runtime_type(member, substitutions))
                .collect(),
        ),
        ir::Type::Tuple(items) => ir::Type::Tuple(
            items
                .iter()
                .map(|item| substitute_runtime_type(item, substitutions))
                .collect(),
        ),
        ir::Type::Record(fields) => ir::Type::Record(
            fields
                .iter()
                .map(|field| ir::NamedType {
                    name: field.name.clone(),
                    ty: substitute_runtime_type(&field.ty, substitutions),
                })
                .collect(),
        ),
        ir::Type::Function { params, ret } => ir::Type::Function {
            params: params
                .iter()
                .map(|param| substitute_runtime_type(param, substitutions))
                .collect(),
            ret: Box::new(substitute_runtime_type(ret, substitutions)),
        },
        _ => ty.clone(),
    }
}

fn runtime_type_kind_name(kind: crate::ast::TypeKind) -> &'static str {
    match kind {
        crate::ast::TypeKind::Annotation => "annotation",
        crate::ast::TypeKind::Class => "class",
        crate::ast::TypeKind::Record => "shape",
        crate::ast::TypeKind::Object => "object",
        crate::ast::TypeKind::Interface => "interface",
        crate::ast::TypeKind::Enum => "union",
    }
}

fn is_primitive_type_name(name: &str) -> bool {
    matches!(name, "Bool" | "Float" | "Int" | "Never" | "Rune" | "Unit")
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
        ir::Type::Union(members) => members
            .iter()
            .map(render_ir_type)
            .collect::<Vec<_>>()
            .join(" | "),
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
            "fn({}) {}",
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
                "field '{}' is provided by multiple spreads; select a value explicitly or use 'override' on one spread",
                name
            ),
        ))
    } else {
        fields.push((name, value));
        Ok(())
    }
}

fn upsert_runtime_record_field(fields: &mut Vec<(String, Value)>, name: String, value: Value) {
    if let Some((_, existing_value)) = fields.iter_mut().find(|(field, _)| field == &name) {
        *existing_value = value;
    } else {
        fields.push((name, value));
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
                private var count Int


                new(count Int) {
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
    fn runs_contextual_empty_brace_construction() {
        let program = lower_inline(
            r#"
            class Counter {
                value Int

                new() {
                    this.value = 7
                }
            }

            class Defaulted {
                value Int = 11
            }

            shape Marker {}

            def noop() Unit = {}
            def noop2() = {}

            def main() Unit {
                counter Counter = {}
                defaulted Defaulted = {}
                marker Marker = {}
                values Vector[Str] = {}
                lookup Map[Str, Int] = {}
                set Set[Str] = {}
                nothing Unit = {}
                anonymous {} = new {}
                structural {} = {}
                defaultedStructural {} = default {}
                widened Any = anonymous
                extended = { ...anonymous, value: 2 }
                positional { x Int, y Int } = new(1, 2)
                reversed { y Int, x Int } = new(3, 4)
                callback fn() Unit = () => {}
                shapeCallback fn() {} = () => new {}
                callbackResult = { ...shapeCallback(), value: 3 }
                callback()
                noop()
                noop2()

                println(counter.value, defaulted.value, marker == Marker())
                println(
                    values.size,
                    lookup.size,
                    set.size,
                    extended.value,
                    callbackResult.value
                )
                println(positional.x, positional.y, reversed.x, reversed.y)
            }
            "#,
        );

        let run = run_program(&program);
        assert!(run.diagnostics.is_empty(), "{:#?}", run.diagnostics);
        assert_eq!(run.output, "7 11 true\n0 0 0 2 3\n1 2 4 3\n");
    }

    #[test]
    fn runs_default_initialization_without_recursive_derivation() {
        let program = lower_inline(
            r#"
            class RetrySettings {
                attempts Int = 3
            }

            class ExplicitSettings {
                attempts Int

                new(attempts Int = 4) {
                    this.attempts = attempts
                }
            }

            shape NestedDefaults {
                value Str = "nested"
            }

            class Config {
                label Str = "kept"
                enabled Bool
                retries RetrySettings
                explicit ExplicitSettings
                nested NestedDefaults
                note Str?
                files [Str]
                counts [Str: Int]
            }

            def main() Unit {
                config Config = default {}
                changed Config = default { label: "changed" }
                anonymous { enabled Bool, files [Str] } = default {}

                println(
                    config.label,
                    config.enabled,
                    config.retries.attempts,
                    config.explicit.attempts,
                    config.nested.value,
                    config.note is None,
                    config.files.size,
                    config.counts.size
                )
                println(changed.label, changed.enabled)
                println(anonymous.enabled, anonymous.files.size)
            }
            "#,
        );

        let run = run_program(&program);
        assert!(run.diagnostics.is_empty(), "{:#?}", run.diagnostics);
        assert_eq!(
            run.output,
            "kept false 3 4 nested true 0 0\nchanged false\nfalse 0\n"
        );
    }

    #[test]
    fn runs_named_declarations_nested_in_other_declarations() {
        let program = lower_inline(
            r#"
            class Namespace {
                shape Point {
                    x Int
                }

                class Worker {
                    name Str

                    def point(x Int) Point = Point(x)
                }

                interface Reader {
                    def read() Str
                }

                object Defaults {
                    prefix Str = "worker:"
                }

                annotation Label {
                    value Str
                }

                def worker(name Str) Worker = Worker(name)
                def point(x Int) Point = Point(x)
                def workerName(worker Worker) Str = match worker {
                    case Worker { name } => name
                }
            }

            def qualifiedWorkerName(worker Namespace.Worker) Str = match worker {
                case Namespace.Worker { name } => name
            }

            class TextReader with Namespace.Reader {
                def read() Str = "!"
            }

            @Namespace.Label { value: "entry" }
            def run() Str {
                namespace = Namespace()
                worker Namespace.Worker = namespace.worker("Ada")
                point Namespace.Point = worker.point(7)
                direct Namespace.Point = Namespace.Point(1)
                return Namespace.Defaults.prefix + namespace.workerName(worker) +
                    qualifiedWorkerName(worker) + (point.x + direct.x).toStr() + TextReader().read()
            }
            "#,
        );

        let run = run_program(&program);
        assert!(run.diagnostics.is_empty(), "{:#?}", run.diagnostics);
        assert_eq!(run.return_value.as_deref(), Some("worker:AdaAda8!"));
    }

    #[test]
    fn runs_nested_type_aliases_and_declared_unions() {
        let program = lower_inline(
            r#"
            class Parser {
                type Text = Str
                type Outcome =
                    class Parsed { text Text }
                    | object Empty {}

                def parse(text Text) Outcome = Outcome.Parsed(text)
            }

            def run() Str {
                outcome Parser.Outcome = Parser().parse("ok")
                return match outcome {
                    case Parsed { text } => text
                    case Empty => "empty"
                }
            }
            "#,
        );

        let run = run_program(&program);
        assert!(run.diagnostics.is_empty(), "{:#?}", run.diagnostics);
        assert_eq!(run.return_value.as_deref(), Some("ok"));
    }

    #[test]
    fn runs_getter_methods_through_member_access() {
        let program = lower_inline(
            r#"
            class Counter {
                value Int

                def doubled Int = this.value * 2
                def quadrupled Int = doubled * 2
            }

            def run() Int {
                counter Counter = Counter(6)
                return counter.quadrupled
            }
            "#,
        );

        let run = run_program(&program);
        assert!(run.diagnostics.is_empty(), "{:#?}", run.diagnostics);
        assert_eq!(run.return_value.as_deref(), Some("24"));
    }

    #[test]
    fn runs_function_values_returned_by_getters() {
        let program = lower_inline(
            r#"
            class Factory {
                def creator fn(Int) Int = (value Int) => value + 1
                def implicitCall Int = creator(4)
            }

            def run() Int {
                factory Factory = Factory()
                return factory.creator(5) + factory.implicitCall
            }
            "#,
        );

        let run = run_program(&program);
        assert!(run.diagnostics.is_empty(), "{:#?}", run.diagnostics);
        assert_eq!(run.return_value.as_deref(), Some("11"));
    }

    #[test]
    fn runs_strict_equality_and_reference_ids() {
        let program = lower_inline(
            r#"
            class Box with Eq[Box] {
                value Int

                def equals(other Box) Bool = this.value == other.value
            }

            def main() Unit {
                first = Box(1)
                alias = first
                separate = Box(1)
                different = Box(2)
                values = [1, 2]
                valuesAlias = values
                valuesCopy = [1, 2]

                OS.println(first === alias)
                OS.println(first === separate)
                OS.println(first !== different)
                OS.println(first.referenceId == alias.referenceId)
                OS.println(first.referenceId == separate.referenceId)
                OS.println(values.referenceId == valuesAlias.referenceId)
                OS.println(values.referenceId == valuesCopy.referenceId)

                visited Set[ReferenceId] = Set()
                visited.add(first.referenceId)
                OS.println(visited.contains(alias.referenceId))
                OS.println(visited.contains(separate.referenceId))
            }
            "#,
        );

        let run = run_program(&program);
        assert!(run.diagnostics.is_empty(), "{:#?}", run.diagnostics);
        assert_eq!(
            run.output,
            "true\ntrue\ntrue\ntrue\nfalse\ntrue\nfalse\ntrue\nfalse\n"
        );
    }

    #[test]
    fn runs_structural_equality_between_distinct_shapes() {
        let program = lower_inline(
            r#"
            shape Point {
                x Int
                label Str
            }

            shape ReorderedPoint {
                label Str
                x Int
            }

            def main() Unit {
                point = Point(1, "one")
                same = ReorderedPoint("one", 1)
                different = ReorderedPoint("two", 2)

                println(point == same)
                println(point != same)
                println(point == different)
                println(same == point)
                println(point.equals(same))
                println(point === same)
                println(point !== different)

                leftAnonymous { x Int, label Str } = { x: 1, label: "one" }
                rightAnonymous { label Str, x Int } = { label: "one", x: 1 }
                println(leftAnonymous == rightAnonymous)
            }
            "#,
        );

        let run = run_program(&program);
        assert!(run.diagnostics.is_empty(), "{:#?}", run.diagnostics);
        assert_eq!(
            run.output,
            "true\nfalse\nfalse\ntrue\ntrue\ntrue\ntrue\ntrue\n"
        );
    }

    #[test]
    fn runs_declared_class_equality_and_any_narrowing() {
        let program = lower_inline(
            r#"
            class Account with Eq[Account] {
                id Int

                def equals(other Account) Bool = this.id == other.id
            }

            interface Identified with Eq[Identified] {
                def code() Int
            }

            class Entry with Identified {
                value Int

                def code() Int = this.value
                def equals(other Identified) Bool = this.value == other.code()
            }

            class AlternateEntry with Identified {
                value Int

                def code() Int = this.value
                def equals(other Identified) Bool = this.value == other.code()
            }

            def main() Unit {
                println(Account(1) == Account(1))
                println(Account(1) != Account(2))
                left Identified = Entry(1)
                right Identified = AlternateEntry(1)
                sameClass Identified = Entry(1)
                different Identified = Entry(2)
                println(left == right)
                println(left === right)
                println(left === sameClass)
                println(left !== different)

                account = Account(3)
                widenedAccount Any = Any(account)
                if let recovered Account = widenedAccount {
                    println(recovered === account)
                }
            }
            "#,
        );

        let run = run_program(&program);
        assert!(run.diagnostics.is_empty(), "{:#?}", run.diagnostics);
        assert_eq!(run.output, "true\ntrue\ntrue\nfalse\ntrue\ntrue\ntrue\n");
    }

    #[test]
    fn uses_declared_class_equality_in_collections_and_nested_values() {
        let program = lower_inline(
            r#"
            class Key with Hashed[Key] {
                id Int
                note Str

                def equals(other Key) Bool = this.id == other.id
                def hash() Int = this.id
            }

            shape WrappedKey {
                key Key
            }

            type KeyChoice =
                shape Chosen { key Key }
                | object Missing {}

            def main() Unit {
                left = Key(1, "left")
                right = Key(1, "right")

                keys Set[Key] = Set()
                keys.add(left)
                keys.add(right)

                lookup [Key: Str] = []
                lookup[left] := "found"

                wrappedLeft = WrappedKey(left)
                wrappedRight = WrappedKey(right)
                wrappedKeys Set[WrappedKey] = Set()
                wrappedKeys.add(wrappedLeft)
                wrappedKeys.add(wrappedRight)

                wrappedLookup [WrappedKey: Str] = []
                wrappedLookup[wrappedLeft] := "wrapped"

                chosenLeft KeyChoice = Chosen(left)
                chosenRight KeyChoice = Chosen(right)

                println(left == right)
                println(keys.size)
                println(lookup[right] ?? "missing")
                println(wrappedLeft == wrappedRight)
                println((left, 2) == (right, 2))
                println(chosenLeft == chosenRight)
                println(wrappedKeys.size)
                println(wrappedLookup[wrappedRight] ?? "missing")
            }
            "#,
        );

        let run = run_program(&program);
        assert!(run.diagnostics.is_empty(), "{:#?}", run.diagnostics);
        assert_eq!(run.output, "true\n1\nfound\ntrue\ntrue\ntrue\n1\nwrapped\n");
    }

    #[test]
    fn runs_local_extension_methods() {
        let program = lower_inline(
            r#"
            class User {
                name Str
                age Int
            }

            ext User {
                def label() Str = this.name + ":" + this.age.toStr()
            }

            def main() Unit {
                user = User { name: "Ada", age: 36 }
                println(user.label())
            }
            "#,
        );

        let run = run_program(&program);
        assert!(run.diagnostics.is_empty(), "{:#?}", run.diagnostics);
        assert_eq!(run.output, "Ada:36\n");
    }

    #[test]
    fn runs_trailing_block_as_explicit_zero_arg_lambda_argument() {
        let program = lower_inline(
            r#"
            def process(f fn() Unit) Unit = f()
            def compute(f fn() Int) Int = f()

            def main() Unit {
                process { () => println("hehe") }
                println(compute { () => 42 })
            }
            "#,
        );

        let run = run_program(&program);
        assert!(run.diagnostics.is_empty(), "{:#?}", run.diagnostics);
        assert_eq!(run.output, "hehe\n42\n");
    }

    #[test]
    fn runs_range_loops_and_println() {
        let program = lower_inline(
            r#"
            def main() Unit {
                range IntRange = Range(1, 4)
                OS.println("bounds", range.start, range.end, range.step)
                var total Int = 0
                for item <- range {
                    OS.println("range", item)
                    total += item
                }
                for item <- range {
                    total += item
                }
                descending IntRange = Range(3, 0)
                OS.println("descending", descending.start, descending.end, descending.step)
                for item <- descending {
                    total += item
                }
                explicit IntRange = IntRange(0, 2, 1)
                for item <- explicit {
                    total += item
                }
                OS.println("total", total)
            }
            "#,
        );

        let run = run_program(&program);
        assert!(run.diagnostics.is_empty(), "{:#?}", run.diagnostics);
        assert_eq!(
            run.output,
            "bounds 1 4 1\nrange 1\nrange 2\nrange 3\ndescending 3 0 -1\ntotal 19\n"
        );
        assert_eq!(run.return_value, None);
    }

    #[test]
    fn runs_defaulted_constructor_parameters() {
        let program = lower_inline(
            r#"
            class User {
                name Str
                age Int


                new(name Str, age Int = 0) {
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
    fn passes_shape_values_as_ordinary_positional_constructor_arguments() {
        let program = lower_inline(
            r#"
            class Holder {
                payload { x Int }
            }

            class ExplicitHolder {
                payload { x Int }

                new(payload { x Int }) {
                    this.payload = payload
                }
            }

            def main() Unit {
                payload = { x: 7 }

                implicitFromValue = Holder(payload)
                implicitFromLiteral = Holder({ x: 8 })
                explicitFromValue = ExplicitHolder(payload)
                explicitFromLiteral = ExplicitHolder({ x: 9 })
                named = Holder { payload }

                println(implicitFromValue.payload.x)
                println(implicitFromLiteral.payload.x)
                println(explicitFromValue.payload.x)
                println(explicitFromLiteral.payload.x)
                println(named.payload.x)
            }
            "#,
        );

        let run = run_program(&program);
        assert!(run.diagnostics.is_empty(), "{:#?}", run.diagnostics);
        assert_eq!(run.output, "7\n8\n7\n9\n7\n");
    }

    #[test]
    fn runs_defaulted_function_and_method_parameters() {
        let program = lower_inline(
            r#"
            def increment(value Int, amount Int = 1) Int = value + amount
            def isMissing(value Int? = None) Bool = value.isEmpty

            class Counter {
                value Int

                new(value Int) {
                    this.value = value
                }

                def add(amount Int = 1) Int = this.value + amount
            }

            def main() Unit {
                counter Counter = Counter(10)
                println(increment(4), increment(4, 3))
                println(counter.add(), counter.add(5))
                println(isMissing(), isMissing(Some(1)))
            }
            "#,
        );

        let run = run_program(&program);
        assert!(run.diagnostics.is_empty(), "{:#?}", run.diagnostics);
        assert_eq!(run.output, "5 7\n11 15\ntrue false\n");
    }

    #[test]
    fn evaluates_receivers_and_explicit_arguments_in_source_order() {
        let program = lower_inline(
            r#"
            class Recorder {
                values [Int] = []

                def mark(value Int) Int {
                    this.values.add(value)
                    value
                }
            }

            class Pair {
                first Int
                second Int
            }

            class ExplicitPair {
                first Int
                second Int

                new(first Int, second Int) {
                    this.first = first
                    this.second = second
                }
            }

            class Service {
                recorder Recorder

                def send(request Int) Int {
                    this.recorder.mark(3)
                    request
                }

                def combine(first Int, second Int) Int {
                    this.recorder.mark(4)
                    first * 10 + second
                }
            }

            class Harness {
                recorder Recorder

                def makeService() Service {
                    this.recorder.mark(1)
                    Service(this.recorder)
                }

                def makeRequest() Int = this.recorder.mark(2)
            }

            def combine(first Int, second Int) Int = first * 10 + second

            def collect(prefix Int, values [Int] vararg) Int =
                prefix + values.fold(0, (sum, value) => sum + value)

            def keep(first Int, second => Int) Int = first

            def main() Unit {
                callRecorder = Recorder {}
                combined = combine(
                    second = callRecorder.mark(2),
                    first = callRecorder.mark(1)
                )
                println(combined, callRecorder.values[0], callRecorder.values[1])

                explicitRecorder = Recorder {}
                explicit Pair = Pair {
                    second: explicitRecorder.mark(2)
                    first: explicitRecorder.mark(1)
                }
                println(explicit.first, explicit.second,
                    explicitRecorder.values[0], explicitRecorder.values[1])

                contextualRecorder = Recorder {}
                contextual Pair = {
                    second: contextualRecorder.mark(2)
                    first: contextualRecorder.mark(1)
                }
                println(contextual.first, contextual.second,
                    contextualRecorder.values[0], contextualRecorder.values[1])

                constructorRecorder = Recorder {}
                constructed ExplicitPair = ExplicitPair {
                    second: constructorRecorder.mark(2)
                    first: constructorRecorder.mark(1)
                }
                println(constructed.first, constructed.second,
                    constructorRecorder.values[0], constructorRecorder.values[1])

                receiverRecorder = Recorder {}
                harness = Harness(receiverRecorder)
                sent = harness.makeService().send(harness.makeRequest())
                println(sent, receiverRecorder.values[0], receiverRecorder.values[1],
                    receiverRecorder.values[2])

                namedReceiverRecorder = Recorder {}
                namedHarness = Harness(namedReceiverRecorder)
                namedResult = namedHarness.makeService().combine(
                    second = namedHarness.makeRequest(),
                    first = namedReceiverRecorder.mark(3)
                )
                println(namedResult, namedReceiverRecorder.values[0],
                    namedReceiverRecorder.values[1], namedReceiverRecorder.values[2],
                    namedReceiverRecorder.values[3])

                variadicRecorder = Recorder {}
                variadicResult = collect(
                    values = [variadicRecorder.mark(2), variadicRecorder.mark(3)],
                    prefix = variadicRecorder.mark(1)
                )
                println(variadicResult, variadicRecorder.values[0],
                    variadicRecorder.values[1], variadicRecorder.values[2])

                lazyRecorder = Recorder {}
                lazyResult = keep(
                    second = panic("must remain lazy"),
                    first = lazyRecorder.mark(1)
                )
                println(lazyResult, lazyRecorder.values[0])
            }
            "#,
        );

        let run = run_program(&program);
        assert!(run.diagnostics.is_empty(), "{:#?}", run.diagnostics);
        assert_eq!(
            run.output,
            "12 2 1\n1 2 2 1\n1 2 2 1\n1 2 2 1\n2 1 2 3\n32 1 2 3 4\n6 2 3 1\n1 1\n"
        );
    }

    #[test]
    fn runs_intermingled_defaults_with_prefix_and_named_calls() {
        let program = lower_inline(
            r#"
            class X1 {
                a Str = "A"
                b Str
            }

            class X2 {
                b Str
                a Str = "A"
            }

            class X3 {
                a Str
                b Str = "B"
                c Str
            }

            class X4 {
                a Str = "A"
                b Str = "B"
            }

            class Article {
                body Str
                title Str

                new(body Str = "body", title Str) {
                    this.body = body
                    this.title = title
                }
            }

            class Client {
                def connect(protocol Str = "https", host Str, port Int = 443) Str =
                    protocol + "://" + host + ":" + port.toStr()
            }

            def connect(protocol Str = "https", host Str, port Int = 443) Str =
                protocol + "://" + host + ":" + port.toStr()

            def main() Unit {
                x1 X1 = X1 { b: "one" }
                x2 X2 = X2("two")
                x3 X3 = X3 { a: "three", c: "four" }
                x3Full X3 = X3("five", "six", "seven")
                x4Empty X4 = X4()
                x4One X4 = X4("eight")
                x4Full X4 = X4("nine", "ten")
                article Article = Article { title: "Intro" }
                client = Client {}

                println(x1.a, x1.b)
                println(x2.b, x2.a)
                println(x3.a, x3.b, x3.c)
                println(x3Full.a, x3Full.b, x3Full.c)
                println(x4Empty.a, x4Empty.b)
                println(x4One.a, x4One.b)
                println(x4Full.a, x4Full.b)
                println(article.body, article.title)
                println(connect(host = "example.com"))
                println(client.connect(host = "example.com", port = 8443))
            }
            "#,
        );

        let run = run_program(&program);
        assert!(run.diagnostics.is_empty(), "{:#?}", run.diagnostics);
        assert_eq!(
            run.output,
            "A one\ntwo A\nthree B four\nfive six seven\nA B\neight B\nnine ten\nbody Intro\nhttps://example.com:443\nhttps://example.com:8443\n"
        );
    }

    #[test]
    fn runs_variadic_constructor_parameters() {
        let program = lower_inline(
            r#"
            class Path {
                segments [Str]


                new(segments [Str] vararg) {
                    this.segments = segments
                }

                def size() Int = this.segments.size

                def firstOr(value Str) Str = this.segments.at(0) ?? value
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
    fn runs_explicit_and_contextual_generic_construction() {
        let program = lower_inline(
            r#"
            class Box[T] {
                value T

                new(value T) {
                    this.value = value
                }
            }

            def main() Unit {
                set Set[Str] = new {}
                map Map[Str, Int] = Map[Str, Int]()
                inferred = Box("hello")
                contextual Box[Str] = new("world")

                println(set.isEmpty, set.nonEmpty)
                set.add("Ada")
                println(set.isEmpty, set.nonEmpty, set.size, map.size, inferred.value, contextual.value)
            }
            "#,
        );

        let run = run_program(&program);
        assert!(run.diagnostics.is_empty(), "{:#?}", run.diagnostics);
        assert_eq!(run.output, "true false\nfalse true 1 0 hello world\n");
    }

    #[test]
    fn runs_explicit_generic_calls_with_complete_type_arguments() {
        let program = lower_inline(
            r#"
            def keepValue[T](value T) T = value

            def main() Unit {
                optional Int? = keepValue[Int?](^5)
                namedOptional Option[Int] = keepValue[Option[Int]](^6)
                callback fn(Int) Int = keepValue[fn(Int) Int]((value Int) => value + 1)

                println(optional!, namedOptional!, callback(41))
            }
            "#,
        );

        let run = run_program(&program);
        assert!(run.diagnostics.is_empty(), "{:#?}", run.diagnostics);
        assert_eq!(run.output, "5 6 42\n");
    }

    #[test]
    fn runs_contextual_shape_projection_inside_generic_map() {
        let program = lower_inline(
            r#"
            shape StoredRollup {
                total Int
                label Str
                internalId Int
            }

            shape Rollup {
                total Int
                label Str
            }

            def source() StoredRollup? = Some(StoredRollup {
                total: 7
                label: "week"
                internalId: 99
            })

            def named() Rollup? = source().map(r => Rollup { ...r })
            def bareFields() Rollup? = source().map(r => { ...r })
            def implicitShape() Rollup? = source().map(r => { ...r })
            def contextualNew() Rollup? = source().map(r => new { ...r })
            def namedMembers() Rollup? = source().map(r => Rollup { total: r.total, label: r.label })
            def bareFieldMembers() Rollup? = source().map(r => { total: r.total, label: r.label })
            def contextualNewMembers() Rollup? = source().map(r => new { total: r.total, label: r.label })

            def main() Unit {
                println(named()!.total)
                println(bareFields()!.total)
                println(implicitShape()!.total)
                println(contextualNew()!.total)
                println(namedMembers()!.total)
                println(bareFieldMembers()!.total)
                println(contextualNewMembers()!.total)
            }
            "#,
        );

        let run = run_program(&program);
        assert!(run.diagnostics.is_empty(), "{:#?}", run.diagnostics);
        assert_eq!(run.output, "7\n7\n7\n7\n7\n7\n7\n");
    }

    #[test]
    fn shape_width_assignment_materializes_projected_shape() {
        let program = lower_inline(
            r#"
            shape Point {
                x Int
                y Int
            }

            shape Point3D {
                x Int
                y Int
                z Int
            }

            shape WiderItems {
                x Int
                items [Int]
                z Int
            }

            shape Items {
                x Int
                items [Int]
            }

            def view(point Point) Point = point

            def main() Unit {
                first3d = Point3D(1, 2, 3)
                second3d = Point3D(1, 2, 99)
                first Point = view(first3d)
                second Point = second3d
                concrete = Point(1, 2)

                println(first == second)
                println(first === second)
                println(first == concrete)
                println(first === concrete)
                println(first.runtimeType.name !)
                match first {
                    case Point3D { z } => println(z)
                    case _ => println("missing")
                }
                if first is Point3D {
                    println(first.z)
                }

                values [Point: Str] = []
                values[first3d] := "first"
                values[second3d] := "second"
                println(values.size, values[first3d]!)

                points Set[Point] = Set()
                points.add(first3d)
                points.add(second3d)
                println(points.size)
                println(values.contains(first3d))

                projected Point = Point { ...first }
                contextual Point = { ...first }
                forced Point = new { ...first }
                updated = first with { x: 4 }
                println(projected === concrete)
                println(contextual === concrete)
                println(forced === concrete)
                println(updated === Point(4, 2))

                wider = WiderItems(1, [1], 3)
                items Items = wider
                wider.items.add(2)
                println(items.items.size)
            }
            "#,
        );

        let run = run_program(&program);
        assert!(run.diagnostics.is_empty(), "{:#?}", run.diagnostics);
        assert_eq!(
            run.output,
            "true\ntrue\ntrue\ntrue\nPoint\nmissing\n1 second\n1\ntrue\ntrue\ntrue\ntrue\ntrue\n2\n"
        );
    }

    #[test]
    fn shape_union_projection_prefers_exact_schema_then_unique_target() {
        let program = lower_inline(
            r#"
            shape X {
                x Int
            }

            shape XY {
                x Int
                y Int
            }

            shape XYZ {
                x Int
                y Int
                z Int
            }

            shape Position {
                y Int
                x Int
            }

            shape Label {
                label Str
            }

            def project(value XYZ | Label) XY | Label = value

            def printSelected(value XY | X) Unit {
                match value {
                    case XY { x, y } => println("xy", x, y)
                    case X { x } => println("x", x)
                }
            }

            def main() Unit {
                wider = XYZ(1, 2, 3)
                unique XY | Label = wider
                exact XY | X = Position(4, 5)
                dynamic = project(wider)

                match unique {
                    case XY { x, y } => println("unique", x, y)
                    case Label { label } => println(label)
                }
                printSelected(exact)
                match dynamic {
                    case XY { x, y } => println("dynamic", x, y)
                    case Label { label } => println(label)
                }
            }
            "#,
        );

        let run = run_program(&program);
        assert!(run.diagnostics.is_empty(), "{:#?}", run.diagnostics);
        assert_eq!(run.output, "unique 1 2\nxy 5 4\ndynamic 1 2\n");
    }

    #[test]
    fn runs_collection_methods_and_core_operator_methods() {
        let program = lower_inline(
            r#"
            class Vec {
                private var items Array[Int]


                new(left Int, right Int) {
                    this.items = Array(left, right)
                }

                def [](index Int) Int = this.items[index]
                def +(other Vec) Vec = Vec(this[0] + other[0], this[1] + other[1])
                def -() Vec = Vec(-this[0], -this[1])
}


            def main() Unit {
                items = Vector(1, 2)
                items.add(3)
                items.addAll(Vector(4, 5))
                OS.println(items[4])

                seen = Set(1, 2)
                seen.add(3)
                OS.println(seen.size)

                pairs = ["a": 1]
                pairs.put("b", 2)
                pairs["a"] += 6
                pairs["a"] -= 2
                OS.println(pairs.size)
                OS.println(pairs["a"]!)

                left Vec = Vec(5, 6)
                OS.println((left + Vec(1, 2))[1])
                OS.println((-left)[0])

                ints = Array.ofInt(2)
                floats = Array.ofFloat(1)
                bools = Array.ofBool(1)
                strs = Array.ofStr(1)
                runes = Array.ofRune(1)
                nul Rune = "\0".runeAt(0)!
                OS.println(ints[0], floats[0], bools[0], strs[0], runes[0] == nul)
            }
            "#,
        );

        let run = run_program(&program);
        assert!(run.diagnostics.is_empty(), "{:#?}", run.diagnostics);
        assert_eq!(run.output, "5\n3\n2\n5\n8\n-5\n0 0.0 false  true\n");
        assert_eq!(run.return_value, None);
    }

    #[test]
    fn preserves_integer_precision_and_defined_numeric_edge_behavior() {
        let program = lower_inline(
            r#"
            def main() Unit {
                low Int = 9007199254740992
                high Int = 9007199254740993

                println(low < high)
                println(high > low)
                println(high <= low)

                maximum Int = 9223372036854775807
                minimum = maximum + 1
                println(minimum)
                println(minimum / -1)
                println(minimum % -1)

                println(1.0 / 0.0 > 0.0)
                println(0.0 / 0.0 < 0.0)
                println(5.5 % 2.0)

                # Mixed comparisons widen Int to Float, so precision may be lost.
                println(high > 9007199254740992.0)
            }
            "#,
        );

        let run = run_program(&program);
        assert!(run.diagnostics.is_empty(), "{:#?}", run.diagnostics);
        assert_eq!(
            run.output,
            "true\ntrue\nfalse\n-9223372036854775808\n-9223372036854775808\n0\ntrue\nfalse\n1.5\nfalse\n"
        );
    }

    #[test]
    fn reports_integer_division_and_remainder_by_zero() {
        for (expression, expected) in [
            ("1 / 0", "integer division by zero"),
            ("1 % 0", "integer remainder by zero"),
        ] {
            let program = lower_inline(&format!(
                r#"
                def main() Unit {{
                    println({expression})
                }}
                "#
            ));

            let run = run_program(&program);
            assert!(
                run.diagnostics
                    .iter()
                    .any(|diagnostic| diagnostic.message.contains(expected)),
                "expected '{expected}', got {:#?}",
                run.diagnostics
            );
        }
    }

    #[test]
    fn runs_list_remove_first_and_seeded_reduce() {
        let program = lower_inline(
            r#"
            def main() Int {
                values = Vector(1, 2, 3)
                values.removeFirst()
                return values.reduce(0, (acc, value) => acc + value)
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
                private map Map[Int, Str] = Map()
                private var currentTick Int = 0
                private queue [Str] = []


                def current() Int = this.currentTick
                def queued() Int = this.queue.size
                def entries() Int = this.map.size
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
    fn runs_stable_implicit_constructor_with_interleaved_non_public_fields() {
        let program = lower_inline(
            r#"
            class Account {
                owner Str
                internal region Str = "US"
                balance Int
                private cache Int = 7

                def label() Str = this.owner + ":" + this.region + ":" + this.balance.toStr()
                def cacheValue() Int = this.cache
            }

            def main() Unit {
                positional = Account("Ada", 10)
                named = Account { owner: "Ben", balance: 20 }
                println(positional.label(), positional.cacheValue())
                println(named.label(), named.cacheValue())
            }
            "#,
        );

        let run = run_program(&program);
        assert!(run.diagnostics.is_empty(), "{:#?}", run.diagnostics);
        assert_eq!(run.output, "Ada:US:10 7\nBen:US:20 7\n");
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
                let Some { value as order } = maybeOrder else panic("expected order")
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
                count = try Some(items.size)
                Some(count)
            }

            def main() Int {
                items = for item <- [1, 2, 3] yield {
                    item + 1
                }

                count = countItems(items) !

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
    fn evaluates_or_pattern_guard_once_per_written_case() {
        let program = lower_inline(
            r#"
            class Guard {
                var calls Int = 0

                def allowed() Bool {
                    this.calls += 1
                    false
                }
            }

            def main() Unit {
                tracker = Guard {}
                result = match (0, 0) {
                    case (0, _) | (_, 0) if tracker.allowed() => "accepted"
                    case _ => "rejected"
                }
                println(result, tracker.calls)
            }
            "#,
        );

        let run = run_program(&program);
        assert!(run.diagnostics.is_empty(), "{:#?}", run.diagnostics);
        assert_eq!(run.output, "rejected 1\n");
    }

    #[test]
    fn runs_negative_numeric_literal_patterns() {
        let program = lower_inline(
            r#"
            shape Reading {
                value Float
            }

            def main() Unit {
                intLabel = match -1 {
                    case -1 => "negative int"
                    case 0 => "zero"
                    case _ => "other"
                }
                floatLabel = match Reading(-3.5) {
                    case Reading { value: -3.5 } => "negative float"
                    case _ => "other"
                }
                OS.println(intLabel)
                OS.println(floatLabel)
            }
            "#,
        );

        let run = run_program(&program);
        assert!(run.diagnostics.is_empty(), "{:#?}", run.diagnostics);
        assert_eq!(run.output, "negative int\nnegative float\n");
    }

    #[test]
    fn runs_for_class_destructuring_with_visible_fields() {
        let program = lower_inline(
            r#"
            class SecretUser {
                name Str
                private token Str
                location Str


                new(name Str, token Str, location Str) {
                    this.name = name
                    this.token = token
                    this.location = location
                }
}


            def main() Unit {
                users = Vector(
                    SecretUser("Sergey", "secret-1", "Tampa"),
                    SecretUser("Ada", "secret-2", "London")
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
                OS.println("some", some ?? 0)
                OS.println("none", none.isEmpty)
                OS.println("ok", ok ?? 0)
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
    fn runs_simple_string_interpolation_identifier_shapes() {
        let program = lower_inline(
            r#"
            def main() Unit {
                user_name Str = "Ada"
                _name Str = "hidden"
                name2 Str = "second"
                OS.println("$user_name $_name $name2 ${user_name} \$user_name")
            }
            "#,
        );

        let run = run_program(&program);
        assert!(run.diagnostics.is_empty(), "{:#?}", run.diagnostics);
        assert_eq!(run.output, "Ada hidden second Ada $user_name\n");
    }

    #[test]
    fn runs_string_rune_access_methods() {
        let program = lower_inline(
            r#"
            def main() Unit {
                word Str = "apple"
                first Rune = word.runeAt(0)!
                let second <- word.runeAt(1) else panic("expected second rune")
                missing = word.runeAt(9)

                OS.println(first)
                OS.println(second)
                OS.println(missing.isEmpty)
                OS.println("😀a".size)
                OS.println("😀a".runeAt(1)!)
            }
            "#,
        );

        let run = run_program(&program);
        assert!(run.diagnostics.is_empty(), "{:#?}", run.diagnostics);
        assert_eq!(run.output, "a\np\ntrue\n2\na\n");
    }

    #[test]
    fn runs_collection_from_add_and_add_all_methods() {
        let program = lower_inline(
            r#"
            def main() Unit {
                base = [1, 2]
                grown = Vector.from(base)
                grown.add(3)
                grown.addAll([4, 5])

                seen = Set(1, 2)
                more = Set.from(seen)
                more.add(3)
                more.addAll([2, 4, 4])

                empty [Int] = []

                OS.println("base", base.size, base.first ?? 0)
                OS.println("grown", grown.size, grown.last ?? 0, grown.contains(4))
                OS.println("empty", empty.last ?? -1)
                OS.println("seen", seen.size, seen.contains(3))
                OS.println("more", more.size, more.contains(4))
            }
            "#,
        );

        let run = run_program(&program);
        assert!(run.diagnostics.is_empty(), "{:#?}", run.diagnostics);
        assert_eq!(
            run.output,
            "base 2 1\ngrown 5 5 true\nempty -1\nseen 2 false\nmore 4 true\n"
        );
    }

    #[test]
    fn runs_linked_list_and_unsafe_extract() {
        let program = lower_inline(
            r#"
            def main() Unit {
                values LinkedList[Int] = LinkedList {}
                values.add(5)
                values.add(8)

                println(values.at(0) !)
                println(Some(13) !)
                println(Ok(21) !)
                println(Right(34) !)
                nested Option[Result[Int, Str]] = Some(Ok(55))
                println(nested!!)
                println(values.removeFirst() !)
                println(values.at(0) !)
            }
            "#,
        );

        let run = run_program(&program);
        assert!(run.diagnostics.is_empty(), "{:#?}", run.diagnostics);
        assert_eq!(run.output, "5\n13\n21\n34\n55\n5\n8\n");
    }

    #[test]
    fn unsafe_extract_panics_on_missing_value() {
        let program = lower_inline(
            r#"
            def main() Unit {
                missing Option[Int] = None
                println(missing !)
            }
            "#,
        );

        let run = run_program(&program);
        assert!(
            run.diagnostics.iter().any(|diag| diag
                .message
                .contains("unsafe extraction expected Option.Some")),
            "{:#?}",
            run.diagnostics
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
                mappedEmpty = empty.map { value => value + 5 }

                values [Int] = [1, 2]
                mapped = values.map { value => value + 5 }

                OS.println(mappedEmpty.size)
                OS.println(mapped.at(0) ?? 0)
                OS.println(mapped.at(1) ?? 0)
                OS.println(mapped.size)
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
            let expected_test_output = parse_comment_block(&text, "# TEST_EXPECT:");
            let expected_failure = parse_comment_block(&text, "# FAIL:");
            let expected_failure_regex = parse_comment_block(&text, "# FAIL_REGEX:");

            if count_defined(&[
                expected_output.is_some(),
                expected_test_output.is_some(),
                expected_failure.is_some(),
                expected_failure_regex.is_some(),
            ]) > 1
            {
                failures.push(format!(
                    "{}\nexample cannot declare more than one of # EXPECT, # TEST_EXPECT, # FAIL, or # FAIL_REGEX",
                    path.strip_prefix(&root).unwrap_or(&path).display()
                ));
                continue;
            }

            if expected_output.is_none()
                && expected_test_output.is_none()
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

            if let Some(expected) = expected_test_output {
                match test_path(&path) {
                    Ok(result) => {
                        if !result.diagnostics.is_empty() {
                            let rendered = result
                                .diagnostics
                                .iter()
                                .map(render_located_diagnostic_for_example)
                                .collect::<Vec<_>>()
                                .join("\n");
                            failures.push(format!(
                                "{}\nexpected test output:\n{}\nactual diagnostics:\n{}",
                                relative, expected, rendered
                            ));
                            continue;
                        }

                        if normalize_example_output(&result.output)
                            != normalize_example_output(&expected)
                        {
                            failures.push(format!(
                                "{}\nexpected test output:\n{}\nactual:\n{}",
                                relative, expected, result.output
                            ));
                        } else {
                            passed.push(relative);
                        }
                    }
                    Err(err) => failures.push(format!(
                        "{}\nexpected test output:\n{}\ntest_path error:\n{}",
                        relative, expected, err
                    )),
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
    fn runs_declared_union_methods_and_default_variant_values() {
        let program = lower_inline(
            r#"
            type Color =
                class Black {
                    color Str = "xxx"
                    temperature Int = 1
                }
                | class Red {
                    color Str = "xxx2"
                    temperature Int = 10
                }

            ext Color {
                def isReddish() Bool = match this {
                    case Black { temperature } => temperature % 5 == 0
                    case Red { temperature } => temperature % 5 == 0
                }
            }

            type OptionX[T] =
                object NoneX {}
                | class SomeX { value T }

            ext OptionX[T] {
                def isDefined() Bool = match this {
                    case SomeX(_) => true
                    case NoneX => false
                }
            }

            def main() Unit {
                black = Color.Black()
                someInt = OptionX.SomeX(5)
                noneInt = OptionX.NoneX

                OS.println("reddish", black.isReddish())
                OS.println("defined", someInt.isDefined)
                OS.println("none", noneInt == OptionX.NoneX)
            }
            "#,
        );

        let run = run_program(&program);
        assert!(run.diagnostics.is_empty(), "{:#?}", run.diagnostics);
        assert_eq!(run.output, "reddish false\ndefined true\nnone true\n");
    }

    #[test]
    fn runs_declared_union_and_object_with_same_name() {
        let program = lower_inline(
            r#"
            type Color =
                object Red {}
                | object Blue {}

            ext Color {
                def label() Str = match this {
                    case Color.Red => "red"
                    case Color.Blue => "blue"
                }
            }

            object Color {


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
    fn runs_impl_object_with_explicit_object_decl() {
        let program = lower_inline(
            r#"
            class Box {
                value Int
            }

            object Box {


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
    fn runs_composed_control_flow_expressions_at_block_tail() {
        let program = lower_inline(
            r#"
            def ifAnswer(flag Bool) Int {
                if flag { 40 } else { 0 } + 2
            }

            def matchAnswer(flag Bool) Int {
                match flag {
                    case true => 40
                    case false => 0
                } + 2
            }

            def main() Unit {
                println(ifAnswer(true), ifAnswer(false))
                println(matchAnswer(true), matchAnswer(false))
            }
            "#,
        );

        let run = run_program(&program);
        assert!(run.diagnostics.is_empty(), "{:#?}", run.diagnostics);
        assert_eq!(run.output, "42 2\n42 2\n");
    }

    #[test]
    fn runs_multiline_type_tests_in_conditions() {
        let program = lower_inline(
            r#"
            def classify(value Any) Int {
                if value is
                    Str {
                    return 1
                }
                if value is
                    not
                    Int {
                    return 2
                }
                3
            }

            def main() Unit {
                println(classify("text"), classify(true), classify(5))
            }
            "#,
        );

        let run = run_program(&program);
        assert!(run.diagnostics.is_empty(), "{:#?}", run.diagnostics);
        assert_eq!(run.output, "1 2 3\n");
    }

    #[test]
    fn runs_empty_lambda_body_after_newline() {
        let program = lower_inline(
            r#"
            def main() Unit {
                sameLine fn() Unit = () => {}
                nextLine fn() Unit = () =>
                    {}

                sameLine()
                nextLine()
                println("complete")
            }
            "#,
        );

        let run = run_program(&program);
        assert!(run.diagnostics.is_empty(), "{:#?}", run.diagnostics);
        assert_eq!(run.output, "complete\n");
    }

    #[test]
    fn runs_parse_and_list_string_helpers() {
        let program = lower_inline(
            r#"
            def main() Unit {
                values = ["btc", "usd"]
                labels = Set("btc", "usd")
                linked = LinkedList("btc", "usd")
                let Some { value as parsedFloat } = Float.parse("1.2") else panic("expected float")
                let Some { value as parsedInt } = Int.parse("7") else panic("expected int")
                OS.println(parsedFloat + 0.8)
                OS.println(parsedInt + 1)
                OS.println(Float.parse("oops").isEmpty)
                OS.println(Int.parse("nope").isEmpty)
                OS.println(values.makeStr("-"))
                OS.println(values.makeStr("/", value => value.toUpper()))
                OS.println(labels.makeStr("/", value => value.toUpper()))
                OS.println(linked.makeStr("/", value => value.toUpper()))
                OS.println(values.nonEmpty)
            }
            "#,
        );

        let run = run_program(&program);
        assert!(run.diagnostics.is_empty(), "{:#?}", run.diagnostics);
        assert_eq!(
            run.output,
            "2.0\n8\ntrue\ntrue\nbtc-usd\nBTC/USD\nBTC/USD\nBTC/USD\ntrue\n"
        );
    }

    #[test]
    fn runs_option_when_static_helper() {
        let program = lower_inline(
            r#"
            def main() Unit {
                someValue = Option.when(true, 7)
                noValue = Option.when(false, 7)
                OS.println(someValue !)
                OS.println(noValue.isEmpty)
            }
            "#,
        );

        let run = run_program(&program);
        assert!(run.diagnostics.is_empty(), "{:#?}", run.diagnostics);
        assert_eq!(run.output, "7\ntrue\n");
    }

    #[test]
    fn runs_match_patterns_for_shapes_classes_and_unions() {
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

            type MaybeInt =
                object NoneX {}
                | class SomeX { value Int }

            def main() Unit {
                amount Amount = Amount(42, "hello")
                pair PairBox = PairBox(5, 9)
                values [MaybeInt] = [MaybeInt.SomeX(1), MaybeInt.NoneX, MaybeInt.SomeX(3)]
                mapped Vector[Option[Int]] = values.map(value => match value {
                    case SomeX { value as x } => Some(x + 1)
                    case NoneX => None
                })

                OS.println(match amount {
                    case Amount { count, label } => count + "-" + label
                })
                OS.println(match pair {
                    case PairBox { left, right } => left + right
                })
                let Some { value as first } = mapped.at(0) else return ()
                let Some { value as second } = mapped.at(1) else return ()
                OS.println(first ?? 0)
                OS.println(second.isEmpty)
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
            shape Amount {
                amount Int
                description Str
                count Int


                def multiple(other Amount) Amount = Amount {
                    amount: this.amount * other.amount
                    description: this.description + " " + other.description
                    count: 0
                }
}


            a1 = Amount(10, "description", 5)
            a2 = a1.multiple(a1)
            a3 = a2 with { amount: 101, description: a2.description + " updated" }
            a4 = (a3 with { amount: 102 }) with { count: 7 }

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
    fn resolves_shape_spread_collisions_at_runtime() {
        let program = lower_inline(
            r#"
            def main() Unit {
                point = { x: 1, y: 2 }
                dot = { x: 3, time: 4 }
                selected = { ...point, ...dot, x: point.x }
                selectedFirst = { x: dot.x, ...point, ...dot }
                overridden = { ...point, override ...dot }

                OS.println(selected.x, selected.y, selected.time)
                OS.println(selectedFirst.x, selectedFirst.y, selectedFirst.time)
                OS.println(overridden.x, overridden.y, overridden.time)
            }
            "#,
        );

        let run = run_program(&program);
        assert!(run.diagnostics.is_empty(), "{:#?}", run.diagnostics);
        assert_eq!(run.output, "1 2 4\n3 2 4\n3 2 4\n");
    }

    #[test]
    fn runs_non_constant_field_initializers_before_new() {
        let program = lower_inline(
            r#"
            class Portfolio {
                assets [Str] = ["btc", "usd"]
                assetCount Int = this.assets.size
                total Int


                new() {
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

            type MaybeApple =
                object NoneX {}
                | class SomeX { value Apple }

            type MaybeAmount =
                object NoneX {}
                | class SomeX { value Amount }

            def main() Unit {
                apple = Apple(12)
                OS.println(match MaybeApple.SomeX(apple) {
                    case SomeX { value: Apple { size } } => "apple " + size
                    case MaybeApple.NoneX => "apple none"
                })
                amount = Amount(13, "cad")
                OS.println(match MaybeAmount.SomeX(amount) {
                    case SomeX { value: Amount { count, label } } => "amount " + count + " " + label
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
    fn runs_composed_control_flow_expressions() {
        let program = lower_inline(
            r#"
            def answer() Int =
                { 40 } + 2

            def render() Str =
                { 42 }.toStr()

            def choose(flag Bool) Int = match flag {
                case true => { 40 } + 2
                case false => 0
            }

            def main() Unit {
                ifValue = if true { 10 } else { 20 } - 1
                matchValue = match false {
                    case true => 10
                    case false => 20
                } - 1
                rightIf = 1 + if true { 2 } else { 3 }

                OS.println(ifValue)
                OS.println(matchValue)
                OS.println(rightIf)
                OS.println(answer())
                OS.println(render())
                OS.println(choose(true))
            }
            "#,
        );

        let run = run_program(&program);
        assert!(run.diagnostics.is_empty(), "{:#?}", run.diagnostics);
        assert_eq!(run.output, "9\n19\n3\n42\n42\n42\n");
    }

    #[test]
    fn runs_mixed_boolean_and_let_condition_clauses() {
        let program = lower_inline(
            r#"
            def main() Unit {
                maybe Int? = Some(4)

                selected = if true && let value <- maybe && value > 3 {
                    value + 1
                } else {
                    0
                }
                OS.println(selected)

                if true && let value <- maybe && value == 4 {
                    OS.println(value)
                }

                var iterations Int = 0
                while iterations < 1 && let value <- maybe && value == 4 {
                    OS.println(value + iterations)
                    iterations += 1
                }

                unknown Any = "lume"
                if true && unknown is Str && unknown.size == 4 {
                    OS.println(unknown)
                }
            }
            "#,
        );

        let run = run_program(&program);
        assert!(run.diagnostics.is_empty(), "{:#?}", run.diagnostics);
        assert_eq!(run.output, "5\n4\n4\nlume\n");
    }

    #[test]
    fn preserves_boolean_precedence_in_control_flow_headers() {
        let program = lower_inline(
            r#"
            def main() Unit {
                a = true
                b = false
                c = false

                if a || b && c {
                    OS.println("if")
                }

                value = if a || b && c { 1 } else { 0 }
                OS.println(value)

                var iterations Int = 0
                while a || b && c {
                    iterations += 1
                    break
                }
                OS.println(iterations)
            }
            "#,
        );

        let run = run_program(&program);
        assert!(run.diagnostics.is_empty(), "{:#?}", run.diagnostics);
        assert_eq!(run.output, "if\n1\n1\n");
    }

    #[test]
    fn runs_if_unwrap_bindings() {
        let program = lower_inline(
            r#"
            def main() Int {
                values = [7]
                if let Some { value } = values.at(0) {
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
                let Pair { left as item, right as other } = pair
                OS.println("let", item + other)

                allSome = [Some(5), Some(6)]
                for maybeItem <- allSome {
                    let Some { value as loopItem } = maybeItem else panic("expected loop item")
                    OS.println("known", loopItem)
                }

                knownMapped = for maybeItem <- allSome yield {
                    let Some { value as mappedItem } = maybeItem else panic("expected mapped item")
                    mappedItem + 1
                }

                for result <- knownMapped {
                    OS.println("known yield", result)
                }

                pairs = Vector(Pair(1, 10), Pair(2, 20), Pair(3, 30))

                for pairItem <- pairs {
                    let Pair { left, right } = pairItem
                    OS.println("for", left, right)
                }

                mapped = for pairItem <- pairs yield {
                    let Pair { left, right } = pairItem
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
    fn runs_let_else_panic_pattern_bindings() {
        let program = lower_inline(
            r#"
            def main() Unit {
                first Option[Int] = Some(5)
                let Some { value } = first else panic("expected first value")
                OS.println("let-panic", value)
            }
            "#,
        );

        let run = run_program(&program);
        assert!(run.diagnostics.is_empty(), "{:#?}", run.diagnostics);
        assert_eq!(run.output, "let-panic 5\n");
    }

    #[test]
    fn runs_assert_runtime_calls() {
        let program = lower_inline(
            r#"
            def main() Unit {
                split = "BTC-USD-5.0".split("-")
                assert(split.size == 3)
                assert(split.size == 3, "split should have 3 parts")
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
                literal Vector[Str] = "a.b".split(".")
                assert(literal.size == 2)
                assert(literal[0] == "a")
                assert(literal[1] == "b")

                split Vector[Str] = "  1234, BUY, 10, NEW  ".trim().splitRegex("\s*,\s*")
                split.add("DONE")
                assert(split.size == 5)
                OS.println(split[0])
                OS.println(split[1])
                OS.println(split[2])
                OS.println(split[3])
                OS.println(split[4])
            }
            "#,
        );

        let run = run_program(&program);
        assert!(run.diagnostics.is_empty(), "{:#?}", run.diagnostics);
        assert_eq!(run.output, "1234\nBUY\n10\nNEW\nDONE\n");
    }

    #[test]
    fn runs_string_empty_checks() {
        let program = lower_inline(
            r#"
            def main() Unit {
                OS.println("".isEmpty)
                OS.println("lume".nonEmpty)
                OS.println("".nonEmpty)
                OS.println("lume".isEmpty)
            }
            "#,
        );

        let run = run_program(&program);
        assert!(run.diagnostics.is_empty(), "{:#?}", run.diagnostics);
        assert_eq!(run.output, "true\ntrue\nfalse\nfalse\n");
    }

    #[test]
    fn runs_trailing_block_lambda_with_multiline_body() {
        let program = lower_inline(
            r#"
            def main() Unit {
                items = [1, 2, 3]
                items.forEach { item =>
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
    fn runs_anonymous_object_interface_methods() {
        let program = lower_inline(
            r#"
            interface Reader {
                def read() Str
            }

            interface Closer {
                def close() Unit
            }

            def main() Unit {
                handler = object with Reader, Closer {
                    source Str = "x"

                    def read() Str = source
                    def close() Unit = OS.println("closed")
                }

                soloReader Reader = object with Reader {
                    source Str = "solo"

                    def read() Str = source
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
    fn runs_union_variant_defaults_and_short_circuit_boolean_ops() {
        let program = lower_inline(
            r#"
            type Outcome =
                class Left {
                    value Str
                    tag Str = "left"
                }
                | object Empty {}

            ext Outcome {
                def tag() Str = match this {
                    case Left { tag } => tag
                    case Empty => ""
                }
            }

            def boom() Bool {
                OS.println("boom")
                true
            }

            def main() Unit {
                left = Outcome.Left("bad")
                if true || boom() {
                    OS.println(left.tag())
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
    fn runs_defer_before_propagating_lifted_failure() {
        let program = lower_inline(
            r#"
            def fail() Result[Int, Str] = Err("failure")

            def load() Result[Int, Str] {
                defer OS.println("cleanup")
                value = try fail()
                Ok(value)
            }

            def main() Unit {
                result Result[Int, Str] = load()
                OS.println(result is Err)
            }
            "#,
        );

        let run = run_program(&program);
        assert!(run.diagnostics.is_empty(), "{:#?}", run.diagnostics);
        assert_eq!(run.output, "cleanup\ntrue\n");
    }

    #[test]
    fn runs_defer_as_lambda_bound() {
        let program = lower_inline(
            r#"
            def main() Unit {
                defer OS.println("main")

                run = () => {
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
    fn runs_all_defers_during_runtime_error_unwinding_and_preserves_primary_error() {
        let program = lower_inline(
            r#"
            def failCleanup() Unit {
                OS.println("failing cleanup")
                panic("cleanup failure")
            }

            def main() Unit {
                defer OS.println("outer cleanup")
                defer failCleanup()
                defer OS.println("inner cleanup")
                panic("body failure")
            }
            "#,
        );

        let run = run_program(&program);
        assert_eq!(run.diagnostics.len(), 1);
        assert!(run.diagnostics[0].message.contains("body failure"));
        assert!(
            run.diagnostics[0]
                .notes
                .iter()
                .any(|note| note.contains("cleanup failure"))
        );
        assert_eq!(
            run.output,
            "inner cleanup\nfailing cleanup\nouter cleanup\n"
        );
    }

    #[test]
    fn reports_cleanup_failure_after_attempting_remaining_defers() {
        let program = lower_inline(
            r#"
            def main() Unit {
                defer OS.println("outer cleanup")
                defer panic("cleanup failure")
                defer OS.println("inner cleanup")
            }
            "#,
        );

        let run = run_program(&program);
        assert_eq!(run.diagnostics.len(), 1);
        assert!(run.diagnostics[0].message.contains("cleanup failure"));
        assert!(
            run.diagnostics[0]
                .notes
                .iter()
                .any(|note| note.contains("deferred cleanup"))
        );
        assert_eq!(run.output, "inner cleanup\nouter cleanup\n");
    }

    #[test]
    fn run_path_executes_plain_module_imports() {
        let path = repo_root().join("examples/imports.lum");
        let run = run_path(path, None).expect("run uses");
        assert!(run.diagnostics.is_empty(), "{:#?}", run.diagnostics);
        assert_eq!(run.output, "hello, Ada\n36\n");
    }

    #[test]
    fn run_path_executes_symbol_and_object_import_forms() {
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

    #[test]
    fn run_path_executes_imported_extension_methods() {
        let path = repo_root().join("examples/extension_methods.lum");
        let run = run_path(path, None).expect("run extension methods");
        assert!(run.diagnostics.is_empty(), "{:#?}", run.diagnostics);
        assert_eq!(run.output, "Ada (36)\ntrue\n");
    }

    #[test]
    fn test_path_discovers_and_runs_specs() {
        let path = repo_root().join("examples/unit_tests.lum");
        let run = test_path(path).expect("run unit tests");
        assert!(run.diagnostics.is_empty(), "{:#?}", run.diagnostics);
        assert_eq!(
            run.output,
            "PASS PrimitiveSpec\nPASS AnotherSpec\n\n2 passed, 0 failed\n"
        );
    }

    #[test]
    fn spec_runner_reports_failures_and_continues() {
        let program = lower_inline(
            r#"
interface Spec {
    def it()
}

class PassingSpec with Spec {
    def it() Unit {}
}

class FailingSpec with Spec {
    def it() Unit {
        panic("expected failure")
    }
}

class LaterSpec with Spec {
    def it() Unit {}
}
"#,
        );
        let run = run_program_specs(&program);
        assert_eq!(run.diagnostics.len(), 1);
        assert!(run.diagnostics[0].message.contains("expected failure"));
        assert_eq!(
            run.output,
            "PASS PassingSpec\nFAIL FailingSpec\nPASS LaterSpec\n\n2 passed, 1 failed\n"
        );
    }
}
