use std::collections::HashSet;

use crate::{ast::*, diagnostic::Diagnostic};

pub(crate) fn member_name(alias: &str, member: &str) -> String {
    format!("__lume_mixin_alias_{}_{alias}_{member}", alias.len())
}

pub(crate) fn member_selector(name: &str) -> Option<(&str, &str)> {
    let rest = name.strip_prefix("__lume_mixin_alias_")?;
    let (length, rest) = rest.split_once('_')?;
    let length = length.parse::<usize>().ok()?;
    Some((rest.get(..length)?, rest.get(length + 1..)?))
}

/// Select original implementations without introducing another receiver. Only
/// class-authored code is rewritten; mixin bodies retain ordinary `this` calls.
pub(crate) fn rewrite(class: &mut TypeDecl) -> Vec<Diagnostic> {
    let aliases: HashSet<_> = class
        .mixins
        .iter()
        .filter_map(|adoption| adoption.alias.as_ref().map(|alias| alias.name.clone()))
        .collect();
    if aliases.is_empty() {
        return Vec::new();
    }
    let mut rewrite = Rewrite {
        aliases,
        diagnostics: Vec::new(),
        receiver_available: true,
    };
    for adoption in &mut class.mixins {
        rewrite.arguments(&mut adoption.args, &HashSet::new());
    }
    for member in &mut class.members {
        match member {
            TypeMember::Method(method) => {
                rewrite.callable(&mut method.params, method.body.as_mut(), &HashSet::new());
            }
            TypeMember::Field(field) => {
                if let Some(value) = &mut field.initializer {
                    rewrite.expression(value, &HashSet::new());
                }
            }
            TypeMember::Case(_) => {}
        }
    }
    rewrite.diagnostics
}

struct Rewrite {
    aliases: HashSet<String>,
    diagnostics: Vec<Diagnostic>,
    receiver_available: bool,
}

impl Rewrite {
    fn selector<'a>(&self, expr: &'a Expr, locals: &HashSet<String>) -> Option<&'a str> {
        match expr {
            Expr::Identifier { name, .. }
                if self.aliases.contains(name) && !locals.contains(name) =>
            {
                Some(name)
            }
            Expr::Group { inner, .. } => self.selector(inner, locals),
            _ => None,
        }
    }

    fn callable(
        &mut self,
        params: &mut [Param],
        body: Option<&mut CallableBody>,
        outer: &HashSet<String>,
    ) {
        let mut locals = outer.clone();
        for param in params {
            if let Some(value) = &mut param.initializer {
                self.expression(value, &locals);
            }
            locals.insert(param.name.clone());
        }
        match body {
            Some(CallableBody::Block(body)) => self.block(body, &locals),
            Some(CallableBody::Expr(value)) => self.expression(value, &locals),
            None => {}
        }
    }

    fn block(&mut self, body: &mut Block, outer: &HashSet<String>) {
        let mut locals = outer.clone();
        for stmt in &mut body.statements {
            match stmt {
                Stmt::Binding(stmt) => {
                    self.expressions(&mut stmt.values, &locals);
                    locals.extend(stmt.bindings.iter().map(|binding| binding.name.clone()));
                }
                Stmt::PatternBinding(stmt) => {
                    self.expression(&mut stmt.value, &locals);
                    self.clauses(&mut stmt.clauses, &mut locals);
                    locals.extend(pattern_binding_names(&stmt.pattern));
                }
                Stmt::LetElse(stmt) => {
                    self.expression(&mut stmt.value, &locals);
                    self.block(&mut stmt.else_block, &locals);
                    self.clauses(&mut stmt.clauses, &mut locals);
                    locals.extend(pattern_binding_names(&stmt.pattern));
                }
                Stmt::Assignment(stmt) => {
                    self.expressions(&mut stmt.targets, &locals);
                    self.expressions(&mut stmt.values, &locals);
                }
                Stmt::Defer(stmt) => match &mut stmt.action {
                    DeferAction::Call(value) => self.expression(value, &locals),
                    DeferAction::Block(body) => self.block(body, &locals),
                },
                Stmt::If(stmt) => self.if_statement(stmt, &locals),
                Stmt::While(stmt) => {
                    let mut inner = locals.clone();
                    self.conditions(&mut stmt.condition_clauses, &mut inner);
                    self.block(&mut stmt.body, &inner);
                }
                Stmt::For(stmt) => {
                    let mut inner = locals.clone();
                    self.generators(&mut stmt.bindings, &mut inner);
                    self.block(&mut stmt.body, &inner);
                }
                Stmt::Match(stmt) => {
                    self.expression(&mut stmt.value, &locals);
                    self.cases(&mut stmt.cases, &locals);
                }
                Stmt::Return(stmt) => {
                    if let Some(value) = &mut stmt.value {
                        self.expression(value, &locals);
                    }
                }
                Stmt::Expr(stmt) => self.expression(&mut stmt.expr, &locals),
                Stmt::LocalFunction(function) => {
                    locals.insert(function.name.clone());
                    self.callable(&mut function.params, Some(&mut function.body), &locals);
                }
                Stmt::Break(_) | Stmt::Continue(_) => {}
            }
        }
    }

    fn clauses(&mut self, clauses: &mut [RefutableClause], locals: &mut HashSet<String>) {
        for clause in clauses {
            self.expression(&mut clause.value, locals);
            locals.extend(pattern_binding_names(&clause.pattern));
        }
    }

    fn conditions(&mut self, clauses: &mut [IfConditionClause], locals: &mut HashSet<String>) {
        for clause in clauses {
            match clause {
                IfConditionClause::Expr(value) => self.expression(value, locals),
                IfConditionClause::Let(clause) => {
                    self.expression(&mut clause.value, locals);
                    locals.extend(pattern_binding_names(&clause.pattern));
                }
            }
        }
    }

    fn if_statement(&mut self, stmt: &mut IfStmt, locals: &HashSet<String>) {
        let mut inner = locals.clone();
        if let Some(value) = &mut stmt.condition {
            self.expression(value, &inner);
        }
        self.conditions(&mut stmt.condition_clauses, &mut inner);
        if let Some(value) = &mut stmt.pattern_value {
            self.expression(value, &inner);
        }
        self.clauses(&mut stmt.pattern_clauses, &mut inner);
        if let Some(pattern) = &stmt.pattern {
            inner.extend(pattern_binding_names(pattern));
        }
        if let Some(value) = &mut stmt.binding_value {
            self.expression(value, &inner);
        }
        inner.extend(stmt.bindings.iter().map(|binding| binding.name.clone()));
        self.block(&mut stmt.then_block, &inner);
        match &mut stmt.else_branch {
            Some(ElseBranch::If(stmt)) => self.if_statement(stmt, locals),
            Some(ElseBranch::Block(body)) => self.block(body, locals),
            None => {}
        }
    }

    fn generators(&mut self, bindings: &mut [ForBinding], locals: &mut HashSet<String>) {
        for binding in bindings {
            if let Some(value) = &mut binding.iterable {
                self.expression(value, locals);
            }
            self.expressions(&mut binding.values, locals);
            locals.extend(binding.bindings.iter().map(|binding| binding.name.clone()));
            if let Some(pattern) = &binding.pattern {
                locals.extend(pattern_binding_names(pattern));
            }
        }
    }

    fn cases(&mut self, cases: &mut [MatchCase], locals: &HashSet<String>) {
        for case in cases {
            let mut inner = locals.clone();
            inner.extend(pattern_binding_names(&case.pattern));
            if let Some(guard) = &mut case.guard {
                self.expression(guard, &inner);
            }
            match &mut case.body {
                MatchCaseBody::Expr(value) => self.expression(value, &inner),
                MatchCaseBody::Block(body) => self.block(body, &inner),
            }
        }
    }

    fn arguments(&mut self, args: &mut [CallArg], locals: &HashSet<String>) {
        for arg in args {
            self.expression(&mut arg.value, locals);
        }
    }

    fn expressions(&mut self, values: &mut [Expr], locals: &HashSet<String>) {
        for value in values {
            self.expression(value, locals);
        }
    }

    fn expression(&mut self, expr: &mut Expr, locals: &HashSet<String>) {
        match expr {
            Expr::Member { receiver, name, .. } => {
                if let Some(alias) = self.selector(receiver, locals) {
                    if self.receiver_available {
                        *name = member_name(alias, name);
                        **receiver = Expr::Identifier {
                            name: "this".into(),
                            span: receiver.span(),
                        };
                        return;
                    }
                }
                self.expression(receiver, locals);
            }
            Expr::Identifier { name, span }
                if self.aliases.contains(name) && !locals.contains(name) =>
            {
                self.diagnostics.push(Diagnostic::error(
                    "invalid_mixin_alias_value",
                    format!("mixin alias '{name}' is an implementation selector, not a value; use '{name}.member' on the consuming class receiver"),
                    *span,
                ));
            }
            Expr::ListLiteral { items, .. } | Expr::TupleLiteral { items, .. } => {
                self.expressions(items, locals);
            }
            Expr::Call { callee, args, .. } => {
                self.expression(callee, locals);
                self.arguments(args, locals);
            }
            Expr::ContextualNew { args, .. } => self.arguments(args, locals),
            Expr::RecordLiteral { fields, values, .. } => {
                self.arguments(fields, locals);
                self.expressions(values, locals);
            }
            Expr::Index {
                receiver, index, ..
            } => {
                self.expression(receiver, locals);
                self.expression(index, locals);
            }
            Expr::Spread { value, .. } | Expr::Try { value, .. } => self.expression(value, locals),
            Expr::Unary { expr, .. } => self.expression(expr, locals),
            Expr::Group { inner, .. } => self.expression(inner, locals),
            Expr::Is { left, .. } => self.expression(left, locals),
            Expr::Binary { left, right, .. } => {
                self.expression(left, locals);
                self.expression(right, locals);
            }
            Expr::ExtractOr {
                value, fallback, ..
            } => {
                self.expression(value, locals);
                self.expression(fallback, locals);
            }
            Expr::Return { value, .. } => {
                if let Some(value) = value {
                    self.expression(value, locals);
                }
            }
            Expr::Block { body, .. } => self.block(body, locals),
            Expr::If {
                condition_clauses,
                then_block,
                else_branch,
                ..
            } => {
                let mut inner = locals.clone();
                self.conditions(condition_clauses, &mut inner);
                self.block(then_block, &inner);
                match else_branch.as_mut() {
                    ElseExprBranch::If(value) => self.expression(value, locals),
                    ElseExprBranch::Block(body) => self.block(body, locals),
                }
            }
            Expr::Match { value, cases, .. } => {
                self.expression(value, locals);
                self.cases(cases, locals);
            }
            Expr::ForYield {
                bindings,
                yield_body,
                ..
            } => {
                let mut inner = locals.clone();
                self.generators(bindings, &mut inner);
                self.block(yield_body, &inner);
            }
            Expr::Lambda { params, body, .. } => {
                let mut inner = locals.clone();
                for param in params {
                    inner.insert(param.name.clone());
                    if let Some(destructure) = &param.destructure {
                        inner.extend(
                            destructure
                                .bindings
                                .iter()
                                .map(|binding| binding.name.clone()),
                        );
                    }
                }
                match body {
                    LambdaBody::Expr(value) => self.expression(value, &inner),
                    LambdaBody::Block(body) => self.block(body, &inner),
                }
            }
            Expr::AnonymousObject {
                fields, methods, ..
            } => {
                let previous = std::mem::replace(&mut self.receiver_available, false);
                for field in fields {
                    if let Some(value) = &mut field.initializer {
                        self.expression(value, locals);
                    }
                }
                for method in methods {
                    self.callable(&mut method.params, method.body.as_mut(), locals);
                }
                self.receiver_available = previous;
            }
            _ => {}
        }
    }
}
