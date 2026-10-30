//! Stateless mixin composition. Bodies become ordinary methods on the one
//! consuming receiver; mixins never acquire storage or runtime identities.

use std::collections::{HashMap, HashSet};

use crate::{ast::*, diagnostic::Diagnostic};

pub(crate) fn method_key(method: &MethodDecl) -> String {
    let generics = method
        .type_params
        .iter()
        .enumerate()
        .map(|(index, param)| (param.name.clone(), format!("@{index}")))
        .collect();
    format!(
        "{}:{}:[{}]({})",
        method.name,
        method.getter,
        method.type_params.len(),
        method
            .params
            .iter()
            .map(|param| format!(
                "{}:{}",
                type_key(param.ty.as_ref(), &generics),
                param.variadic
            ))
            .collect::<Vec<_>>()
            .join(",")
    )
}

fn composition_key(method: &MethodDecl, aliases: &HashMap<String, TypeAliasDecl>) -> String {
    let mut method = method.clone();
    let mut aliases = aliases.clone();
    for param in &method.type_params {
        aliases.remove(&param.name);
    }
    for param in &mut method.params {
        if let Some(ty) = &mut param.ty {
            expand_aliases(ty, &aliases, &mut HashSet::new());
        }
    }
    method_key(&method)
}

fn expand_aliases(
    ty: &mut TypeRef,
    aliases: &HashMap<String, TypeAliasDecl>,
    seen: &mut HashSet<String>,
) {
    match ty {
        TypeRef::Named { name, args, .. } => {
            for arg in args.iter_mut() {
                expand_aliases(arg, aliases, seen);
            }
            if let Some(alias) = aliases.get(name)
                && args.len() == alias.type_params.len()
                && seen.insert(name.clone())
            {
                let name = name.clone();
                let subst = alias
                    .type_params
                    .iter()
                    .map(|param| param.name.clone())
                    .zip(args.iter().cloned())
                    .collect();
                *ty = alias.target.clone();
                substitute(ty, &subst);
                expand_aliases(ty, aliases, seen);
                seen.remove(&name);
            }
        }
        TypeRef::Tuple { fields, .. } => {
            for field in fields {
                expand_aliases(&mut field.ty, aliases, seen);
            }
        }
        TypeRef::Record { fields, .. } => {
            for field in fields {
                expand_aliases(&mut field.ty, aliases, seen);
            }
        }
        TypeRef::Function { params, ret, .. } => {
            for param in params {
                expand_aliases(param, aliases, seen);
            }
            expand_aliases(ret, aliases, seen);
        }
        TypeRef::Union { members, .. } => {
            for member in members {
                expand_aliases(member, aliases, seen);
            }
        }
        TypeRef::Wildcard { .. } => {}
    }
}

fn type_key(ty: Option<&TypeRef>, generics: &HashMap<String, String>) -> String {
    match ty {
        None => "?".into(),
        Some(TypeRef::Wildcard { .. }) => "_".into(),
        Some(TypeRef::Named { name, args, .. }) => format!(
            "{}[{}]",
            generics.get(name).unwrap_or(name),
            args.iter()
                .map(|arg| type_key(Some(arg), generics))
                .collect::<Vec<_>>()
                .join(",")
        ),
        Some(TypeRef::Tuple { fields, .. }) => format!(
            "({})",
            fields
                .iter()
                .map(|field| type_key(Some(&field.ty), generics))
                .collect::<Vec<_>>()
                .join(",")
        ),
        Some(TypeRef::Record { fields, .. }) => {
            let mut keys = fields
                .iter()
                .map(|field| format!("{}:{}", field.name, type_key(Some(&field.ty), generics)))
                .collect::<Vec<_>>();
            keys.sort();
            format!("{{{}}}", keys.join(","))
        }
        Some(TypeRef::Function { params, ret, .. }) => format!(
            "fn({}){}",
            params
                .iter()
                .map(|param| type_key(Some(param), generics))
                .collect::<Vec<_>>()
                .join(","),
            type_key(Some(ret), generics)
        ),
        Some(TypeRef::Union { members, .. }) => {
            let mut keys = members
                .iter()
                .map(|member| type_key(Some(member), generics))
                .collect::<Vec<_>>();
            keys.sort();
            keys.join("|")
        }
    }
}

pub(crate) fn compose(program: &Program) -> Result<Program, Vec<Diagnostic>> {
    let aliases = program
        .items
        .iter()
        .filter_map(|item| match item {
            Item::TypeAlias(alias) => Some((alias.name.clone(), alias.clone())),
            _ => None,
        })
        .collect::<HashMap<_, _>>();
    let declarations = program
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Type(decl) if decl.kind == TypeKind::Mixin => Some((decl.name.clone(), decl)),
            _ => None,
        })
        .collect::<HashMap<_, _>>();
    let mut result = program.clone();
    let mut diagnostics = Vec::new();
    for item in &mut result.items {
        let Item::Type(class) = item else { continue };
        if class.kind != TypeKind::Class {
            continue;
        }
        let mut aliases = aliases.clone();
        for param in &class.type_params {
            aliases.remove(&param.name);
        }
        let mut selected = class
            .members
            .iter()
            .filter_map(|member| match member {
                TypeMember::Method(method) => Some(composition_key(method, &aliases)),
                _ => None,
            })
            .collect::<HashSet<_>>();
        for target in std::mem::take(&mut class.mixins) {
            let TypeRef::Named { name, args, span } = target else {
                continue;
            };
            let mut candidate = Some(class.name.as_str());
            let mut declaration = declarations.get(&name).copied();
            while declaration.is_none() {
                let Some(owner) = candidate else { break };
                declaration = declarations.get(&format!("{owner}.{name}")).copied();
                candidate = owner.rsplit_once('.').map(|(parent, _)| parent);
            }
            let Some(mixin) = declaration else {
                diagnostics.push(Diagnostic::error(
                    "invalid_mixin_use",
                    format!("mixin '{name}' was not resolved before composition"),
                    span,
                ));
                continue;
            };
            let subst = mixin
                .type_params
                .iter()
                .map(|param| param.name.clone())
                .zip(args)
                .collect();
            for member in &mixin.members {
                let TypeMember::Method(method) = member else {
                    continue;
                };
                let method = instantiate_method(method, &subst);
                if selected.insert(composition_key(&method, &aliases)) {
                    class.members.push(TypeMember::Method(method));
                }
            }
        }
    }
    result
        .items
        .retain(|item| !matches!(item, Item::Type(decl) if decl.kind == TypeKind::Mixin));
    if diagnostics.is_empty() {
        Ok(result)
    } else {
        Err(diagnostics)
    }
}

pub(crate) fn instantiate_method(
    method: &MethodDecl,
    subst: &HashMap<String, TypeRef>,
) -> MethodDecl {
    let mut result = method.clone();
    let mut subst = subst.clone();
    for param in &result.type_params {
        subst.remove(&param.name);
    }
    for param in &mut result.type_params {
        for bound in &mut param.bounds {
            substitute(bound, &subst);
        }
    }
    for condition in &mut result.type_conditions {
        match condition {
            GenericCondition::Bound { subject, bound, .. } => {
                substitute(subject, &subst);
                substitute(bound, &subst);
            }
            GenericCondition::Equal { left, right, .. } => {
                substitute(left, &subst);
                substitute(right, &subst);
            }
        }
    }
    for param in &mut result.params {
        optional_type(&mut param.ty, &subst);
        if let Some(expr) = &mut param.initializer {
            expression(expr, &subst);
        }
    }
    optional_type(&mut result.return_type, &subst);
    if let Some(body) = &mut result.body {
        match body {
            CallableBody::Block(body) => block(body, &subst),
            CallableBody::Expr(expr) => expression(expr, &subst),
        }
    }
    for annotation in &mut result.annotations {
        expression(&mut annotation.value, &subst);
    }
    result
}

pub(crate) fn substitute(ty: &mut TypeRef, subst: &HashMap<String, TypeRef>) {
    match ty {
        TypeRef::Named { name, args, .. } => {
            if args.is_empty()
                && let Some(value) = subst.get(name)
            {
                *ty = value.clone();
            } else {
                for arg in args {
                    substitute(arg, subst);
                }
            }
        }
        TypeRef::Tuple { fields, .. } => {
            for field in fields {
                substitute(&mut field.ty, subst);
            }
        }
        TypeRef::Record { fields, .. } => {
            for field in fields {
                substitute(&mut field.ty, subst);
            }
        }
        TypeRef::Function { params, ret, .. } => {
            for param in params {
                substitute(param, subst);
            }
            substitute(ret, subst);
        }
        TypeRef::Union { members, .. } => {
            for member in members {
                substitute(member, subst);
            }
        }
        TypeRef::Wildcard { .. } => {}
    }
}

fn optional_type(ty: &mut Option<TypeRef>, subst: &HashMap<String, TypeRef>) {
    if let Some(ty) = ty {
        substitute(ty, subst);
    }
}

fn pattern(value: &mut Pattern, subst: &HashMap<String, TypeRef>) {
    match value {
        Pattern::Type { target, .. } => substitute(target, subst),
        Pattern::Extract { inner, .. } | Pattern::Alias { inner, .. } => pattern(inner, subst),
        Pattern::Tuple { elements, .. } | Pattern::List { elements, .. } => {
            for item in elements {
                pattern(item, subst);
            }
        }
        Pattern::Record { fields, .. } => {
            for field in fields {
                pattern(&mut field.pattern, subst);
            }
        }
        Pattern::Constructor { args, .. } => {
            for arg in args {
                pattern(arg, subst);
            }
        }
        Pattern::Literal { value, .. } => expression(value, subst),
        Pattern::Wildcard { .. } | Pattern::Binding { .. } => {}
    }
}

fn clause(clause: &mut RefutableClause, subst: &HashMap<String, TypeRef>) {
    pattern(&mut clause.pattern, subst);
    expression(&mut clause.value, subst);
}

fn conditions(clauses: &mut [IfConditionClause], subst: &HashMap<String, TypeRef>) {
    for item in clauses {
        match item {
            IfConditionClause::Expr(expr) => expression(expr, subst),
            IfConditionClause::Let(value) => clause(value, subst),
        }
    }
}

fn for_bindings(bindings: &mut [ForBinding], subst: &HashMap<String, TypeRef>) {
    for binding in bindings {
        for local in &mut binding.bindings {
            optional_type(&mut local.ty, subst);
        }
        if let Some(value) = &mut binding.pattern {
            pattern(value, subst);
        }
        if let Some(value) = &mut binding.iterable {
            expression(value, subst);
        }
        for value in &mut binding.values {
            expression(value, subst);
        }
    }
}

fn cases(cases: &mut [MatchCase], subst: &HashMap<String, TypeRef>) {
    for case in cases {
        pattern(&mut case.pattern, subst);
        if let Some(guard) = &mut case.guard {
            expression(guard, subst);
        }
        match &mut case.body {
            MatchCaseBody::Block(body) => block(body, subst),
            MatchCaseBody::Expr(expr) => expression(expr, subst),
        }
    }
}

fn if_statement(stmt: &mut IfStmt, subst: &HashMap<String, TypeRef>) {
    if let Some(value) = &mut stmt.condition {
        expression(value, subst);
    }
    conditions(&mut stmt.condition_clauses, subst);
    if let Some(value) = &mut stmt.pattern {
        pattern(value, subst);
    }
    if let Some(value) = &mut stmt.pattern_value {
        expression(value, subst);
    }
    for value in &mut stmt.pattern_clauses {
        clause(value, subst);
    }
    for binding in &mut stmt.bindings {
        optional_type(&mut binding.ty, subst);
    }
    if let Some(value) = &mut stmt.binding_value {
        expression(value, subst);
    }
    block(&mut stmt.then_block, subst);
    if let Some(branch) = &mut stmt.else_branch {
        match branch {
            ElseBranch::If(stmt) => if_statement(stmt, subst),
            ElseBranch::Block(body) => block(body, subst),
        }
    }
}

fn block(body: &mut Block, subst: &HashMap<String, TypeRef>) {
    for stmt in &mut body.statements {
        match stmt {
            Stmt::Binding(stmt) => {
                for binding in &mut stmt.bindings {
                    optional_type(&mut binding.ty, subst);
                }
                for value in &mut stmt.values {
                    expression(value, subst);
                }
            }
            Stmt::Assignment(stmt) => {
                for value in stmt.targets.iter_mut().chain(&mut stmt.values) {
                    expression(value, subst);
                }
            }
            Stmt::PatternBinding(stmt) => {
                pattern(&mut stmt.pattern, subst);
                expression(&mut stmt.value, subst);
                for item in &mut stmt.clauses {
                    clause(item, subst);
                }
            }
            Stmt::LetElse(stmt) => {
                pattern(&mut stmt.pattern, subst);
                expression(&mut stmt.value, subst);
                for item in &mut stmt.clauses {
                    clause(item, subst);
                }
                block(&mut stmt.else_block, subst);
            }
            Stmt::Defer(stmt) => match &mut stmt.action {
                DeferAction::Call(expr) => expression(expr, subst),
                DeferAction::Block(body) => block(body, subst),
            },
            Stmt::If(stmt) => if_statement(stmt, subst),
            Stmt::While(stmt) => {
                conditions(&mut stmt.condition_clauses, subst);
                block(&mut stmt.body, subst);
            }
            Stmt::For(stmt) => {
                for_bindings(&mut stmt.bindings, subst);
                block(&mut stmt.body, subst);
            }
            Stmt::Match(stmt) => {
                expression(&mut stmt.value, subst);
                cases(&mut stmt.cases, subst);
            }
            Stmt::Return(stmt) => {
                if let Some(value) = &mut stmt.value {
                    expression(value, subst);
                }
            }
            Stmt::Expr(stmt) => expression(&mut stmt.expr, subst),
            Stmt::LocalFunction(function) => {
                let method = MethodDecl {
                    annotations: function.annotations.clone(),
                    visibility: function.visibility,
                    name: function.name.clone(),
                    getter: false,
                    type_params: function.type_params.clone(),
                    type_conditions: function.type_conditions.clone(),
                    params: function.params.clone(),
                    return_type: function.return_type.clone(),
                    body: Some(function.body.clone()),
                    span: function.span,
                };
                let method = instantiate_method(&method, subst);
                function.params = method.params;
                function.return_type = method.return_type;
                function.body = method.body.unwrap();
                function.type_params = method.type_params;
                function.type_conditions = method.type_conditions;
                function.annotations = method.annotations;
            }
            Stmt::Break(_) | Stmt::Continue(_) => {}
        }
    }
}

fn arguments(args: &mut [CallArg], subst: &HashMap<String, TypeRef>) {
    for arg in args {
        optional_type(&mut arg.ty, subst);
        expression(&mut arg.value, subst);
    }
}

fn expression(expr: &mut Expr, subst: &HashMap<String, TypeRef>) {
    match expr {
        Expr::Identifier { .. }
        | Expr::Placeholder { .. }
        | Expr::Integer { .. }
        | Expr::Float { .. }
        | Expr::Decimal { .. }
        | Expr::String { .. }
        | Expr::Bool { .. }
        | Expr::Unit { .. }
        | Expr::Break { .. }
        | Expr::Continue { .. } => {}
        Expr::Member { receiver, .. } => expression(receiver, subst),
        Expr::Spread { value, .. }
        | Expr::Try { value, .. }
        | Expr::Unary { expr: value, .. }
        | Expr::Group { inner: value, .. } => expression(value, subst),
        Expr::Return { value, .. } => {
            if let Some(value) = value {
                expression(value, subst);
            }
        }
        Expr::Binary { left, right, .. }
        | Expr::ExtractOr {
            value: left,
            fallback: right,
            ..
        } => {
            expression(left, subst);
            expression(right, subst);
        }
        Expr::ListLiteral { items, .. } | Expr::TupleLiteral { items, .. } => {
            for item in items {
                expression(item, subst);
            }
        }
        Expr::Call { callee, args, .. } => {
            expression(callee, subst);
            arguments(args, subst);
        }
        Expr::ContextualNew { args, .. } => arguments(args, subst),
        Expr::RecordLiteral { fields, values, .. } => {
            arguments(fields, subst);
            for value in values {
                expression(value, subst);
            }
        }
        Expr::Index {
            receiver,
            index,
            explicit_type_args,
            ..
        } => {
            expression(receiver, subst);
            expression(index, subst);
            if let Some(args) = explicit_type_args {
                for arg in args {
                    substitute(arg, subst);
                }
            }
        }
        Expr::Is { left, target, .. } => {
            expression(left, subst);
            substitute(target, subst);
        }
        Expr::TypeOf { ty, .. } => substitute(ty, subst),
        Expr::Block { body, .. } => block(body, subst),
        Expr::If {
            condition_clauses,
            then_block,
            else_branch,
            ..
        } => {
            conditions(condition_clauses, subst);
            block(then_block, subst);
            match else_branch.as_mut() {
                ElseExprBranch::If(expr) => expression(expr, subst),
                ElseExprBranch::Block(body) => block(body, subst),
            }
        }
        Expr::Match {
            value, cases: arms, ..
        } => {
            expression(value, subst);
            cases(arms, subst);
        }
        Expr::ForYield {
            bindings,
            yield_body,
            ..
        } => {
            for_bindings(bindings, subst);
            block(yield_body, subst);
        }
        Expr::Lambda { params, body, .. } => {
            for param in params {
                optional_type(&mut param.ty, subst);
            }
            match body {
                LambdaBody::Expr(expr) => expression(expr, subst),
                LambdaBody::Block(body) => block(body, subst),
            }
        }
        Expr::AnonymousObject {
            interfaces,
            fields,
            methods,
            ..
        } => {
            for interface in interfaces {
                substitute(interface, subst);
            }
            for field in fields {
                optional_type(&mut field.ty, subst);
                if let Some(expr) = &mut field.initializer {
                    expression(expr, subst);
                }
            }
            for method in methods {
                *method = instantiate_method(method, subst);
            }
        }
    }
}
