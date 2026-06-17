//! Body-level desugaring into the first core language slice.
//!
//! This pass is intentionally small for now. It normalizes a few surface-only
//! details while keeping most control-flow structure intact so the project can
//! introduce Core incrementally.

use crate::{ast, core};

pub fn desugar_function_decl(function: &ast::FunctionDecl) -> core::FunctionDecl {
    core::FunctionDecl {
        annotations: function.annotations.clone(),
        visibility: function.visibility,
        name: function.name.clone(),
        type_params: function.type_params.clone(),
        params: function.params.clone(),
        return_type: function.return_type.clone(),
        body: desugar_callable_body(&function.body),
        span: function.span,
    }
}

pub fn desugar_method_decl(method: &ast::MethodDecl) -> core::MethodDecl {
    core::MethodDecl {
        annotations: method.annotations.clone(),
        visibility: method.visibility,
        name: method.name.clone(),
        type_params: method.type_params.clone(),
        params: method.params.clone(),
        return_type: method.return_type.clone(),
        body: method.body.as_ref().map(desugar_callable_body),
        span: method.span,
    }
}

pub fn desugar_callable_body(body: &ast::CallableBody) -> core::CallableBody {
    match body {
        ast::CallableBody::Block(block) => core::CallableBody::Block(desugar_block(block)),
        ast::CallableBody::Expr(expr) => core::CallableBody::Expr(desugar_expr(expr)),
    }
}

pub fn desugar_block(block: &ast::Block) -> core::Block {
    core::Block {
        statements: block.statements.iter().map(desugar_stmt).collect(),
        span: block.span,
    }
}

pub fn desugar_stmt(stmt: &ast::Stmt) -> core::Stmt {
    match stmt {
        ast::Stmt::Binding(stmt) => core::Stmt::Binding(core::BindingStmt {
            visibility: stmt.visibility,
            bindings: stmt.bindings.clone(),
            values: stmt.values.iter().map(desugar_expr).collect(),
            destructure: stmt.destructure,
            span: stmt.span,
        }),
        ast::Stmt::PatternBinding(stmt) => core::Stmt::PatternBinding(core::PatternBindingStmt {
            kind: match stmt.kind {
                ast::PatternBindingKind::Let => core::PatternBindingKind::Let,
                ast::PatternBindingKind::Expect => core::PatternBindingKind::Expect,
            },
            clauses: stmt.clauses.iter().map(desugar_refutable_clause).collect(),
            pattern: stmt.pattern.clone(),
            value: desugar_expr(&stmt.value),
            span: stmt.span,
        }),
        ast::Stmt::Assignment(stmt) => core::Stmt::Assignment(core::AssignmentStmt {
            targets: stmt.targets.iter().map(desugar_expr).collect(),
            operator: stmt.operator,
            values: stmt.values.iter().map(desugar_expr).collect(),
            span: stmt.span,
        }),
        ast::Stmt::If(stmt) => core::Stmt::If(core::IfStmt {
            condition: stmt.condition.as_ref().map(desugar_expr),
            condition_clauses: stmt
                .condition_clauses
                .iter()
                .map(desugar_if_condition_clause)
                .collect(),
            pattern: stmt.pattern.clone(),
            pattern_value: stmt.pattern_value.as_ref().map(desugar_expr),
            pattern_clauses: stmt
                .pattern_clauses
                .iter()
                .map(desugar_refutable_clause)
                .collect(),
            bindings: stmt.bindings.clone(),
            binding_value: stmt.binding_value.as_ref().map(desugar_expr),
            then_block: desugar_block(&stmt.then_block),
            else_branch: stmt.else_branch.as_ref().map(desugar_else_branch),
            span: stmt.span,
        }),
        ast::Stmt::Match(stmt) => core::Stmt::Match(core::MatchStmt {
            partial: stmt.partial,
            value: desugar_expr(&stmt.value),
            cases: stmt.cases.iter().map(desugar_match_case).collect(),
            span: stmt.span,
        }),
        ast::Stmt::While(stmt) => core::Stmt::While(core::WhileStmt {
            condition: desugar_expr(&stmt.condition),
            body: desugar_block(&stmt.body),
            span: stmt.span,
        }),
        ast::Stmt::For(stmt) => core::Stmt::For(core::ForStmt {
            bindings: stmt.bindings.iter().map(desugar_for_binding).collect(),
            body: desugar_block(&stmt.body),
            span: stmt.span,
        }),
        ast::Stmt::LetElse(stmt) => core::Stmt::LetElse(core::LetElseStmt {
            clauses: stmt.clauses.iter().map(desugar_refutable_clause).collect(),
            pattern: stmt.pattern.clone(),
            value: desugar_expr(&stmt.value),
            else_block: desugar_block(&stmt.else_block),
            span: stmt.span,
        }),
        ast::Stmt::Return(stmt) => core::Stmt::Return(core::ReturnStmt {
            value: stmt.value.as_ref().map(desugar_expr),
            span: stmt.span,
        }),
        ast::Stmt::Break(stmt) => core::Stmt::Break(core::BreakStmt { span: stmt.span }),
        ast::Stmt::Continue(stmt) => core::Stmt::Continue(core::ContinueStmt { span: stmt.span }),
        ast::Stmt::Expr(stmt) => core::Stmt::Expr(core::ExprStmt {
            expr: desugar_expr(&stmt.expr),
            span: stmt.span,
        }),
        ast::Stmt::LocalFunction(function) => {
            core::Stmt::LocalFunction(desugar_function_decl(function))
        }
    }
}

pub fn desugar_expr(expr: &ast::Expr) -> core::Expr {
    match expr {
        ast::Expr::Identifier { name, span } => core::Expr::Identifier {
            name: name.clone(),
            span: *span,
        },
        ast::Expr::Placeholder { span } => core::Expr::Placeholder { span: *span },
        ast::Expr::Integer { raw, span } => core::Expr::Integer {
            raw: raw.clone(),
            span: *span,
        },
        ast::Expr::Float { raw, span } => core::Expr::Float {
            raw: raw.clone(),
            span: *span,
        },
        ast::Expr::String { raw, span } => core::Expr::String {
            raw: raw.clone(),
            span: *span,
        },
        ast::Expr::Bool { value, span } => core::Expr::Bool {
            value: *value,
            span: *span,
        },
        ast::Expr::Unit { span } => core::Expr::Unit { span: *span },
        ast::Expr::ListLiteral { items, span } => core::Expr::ListLiteral {
            items: items.iter().map(desugar_expr).collect(),
            span: *span,
        },
        ast::Expr::TupleLiteral { items, span } => core::Expr::TupleLiteral {
            items: items.iter().map(desugar_expr).collect(),
            span: *span,
        },
        ast::Expr::Call {
            callee,
            args,
            uses_brace_syntax,
            span,
        } => {
            let style = if *uses_brace_syntax {
                core::CallStyle::Brace
            } else {
                core::CallStyle::Paren
            };
            core::Expr::Call {
                callee: Box::new(desugar_expr(callee)),
                args: args
                    .iter()
                    .map(|arg| desugar_call_arg(arg, style))
                    .collect(),
                style,
                span: *span,
            }
        }
        ast::Expr::Member {
            receiver,
            name,
            span,
        } => core::Expr::Member {
            receiver: Box::new(desugar_expr(receiver)),
            name: name.clone(),
            span: *span,
        },
        ast::Expr::Index {
            receiver,
            index,
            span,
        } => core::Expr::Index {
            receiver: Box::new(desugar_expr(receiver)),
            index: Box::new(desugar_expr(index)),
            span: *span,
        },
        ast::Expr::RecordUpdate {
            receiver,
            updates,
            span,
        } => core::Expr::RecordUpdate {
            receiver: Box::new(desugar_expr(receiver)),
            updates: updates
                .iter()
                .map(|arg| core::CallArg {
                    name: arg.name.clone(),
                    value: desugar_expr(&arg.value),
                    span: arg.span,
                })
                .collect(),
            span: *span,
        },
        ast::Expr::RecordLiteral {
            fields,
            values,
            span,
        } => core::Expr::RecordLiteral {
            fields: fields
                .iter()
                .map(|arg| core::CallArg {
                    name: arg.name.clone(),
                    value: desugar_expr(&arg.value),
                    span: arg.span,
                })
                .collect(),
            values: values.iter().map(desugar_expr).collect(),
            span: *span,
        },
        ast::Expr::AnonymousInterface {
            interfaces,
            methods,
            span,
        } => core::Expr::AnonymousInterface {
            interfaces: interfaces.clone(),
            methods: methods.iter().map(desugar_method_decl).collect(),
            span: *span,
        },
        ast::Expr::Try { value, span } => core::Expr::Try {
            value: Box::new(desugar_expr(value)),
            span: *span,
        },
        ast::Expr::Unary { op, expr, span } => core::Expr::Unary {
            op: *op,
            expr: Box::new(desugar_expr(expr)),
            span: *span,
        },
        ast::Expr::Binary {
            left,
            op,
            right,
            span,
        } => core::Expr::Binary {
            left: Box::new(desugar_expr(left)),
            op: *op,
            right: Box::new(desugar_expr(right)),
            span: *span,
        },
        ast::Expr::Is { left, target, span } => core::Expr::Is {
            left: Box::new(desugar_expr(left)),
            target: target.clone(),
            span: *span,
        },
        ast::Expr::If {
            condition,
            then_block,
            else_branch,
            span,
        } => core::Expr::If {
            condition: Box::new(desugar_expr(condition)),
            then_block: desugar_block(then_block),
            else_branch: Box::new(desugar_else_expr_branch(else_branch)),
            span: *span,
        },
        ast::Expr::Block { body, span } => core::Expr::Block {
            body: desugar_block(body),
            span: *span,
        },
        ast::Expr::Match {
            partial,
            value,
            cases,
            span,
        } => core::Expr::Match {
            partial: *partial,
            value: Box::new(desugar_expr(value)),
            cases: cases.iter().map(desugar_match_case).collect(),
            span: *span,
        },
        ast::Expr::ForYield {
            bindings,
            yield_body,
            span,
        } => core::Expr::ForYield {
            bindings: bindings.iter().map(desugar_for_binding).collect(),
            yield_body: desugar_block(yield_body),
            span: *span,
        },
        ast::Expr::Lambda { params, body, span } => core::Expr::Lambda {
            params: params.clone(),
            body: Box::new(desugar_lambda_body(body)),
            span: *span,
        },
        ast::Expr::Group { inner, .. } => desugar_expr(inner),
    }
}

fn desugar_lambda_body(body: &ast::LambdaBody) -> core::Expr {
    match body {
        ast::LambdaBody::Expr(expr) => desugar_expr(expr),
        ast::LambdaBody::Block(block) => core::Expr::Block {
            body: desugar_block(block),
            span: block.span,
        },
    }
}

fn desugar_call_arg(arg: &ast::CallArg, style: core::CallStyle) -> core::CallArg {
    let value = normalize_brace_lambda_arg(style, desugar_expr(&arg.value));
    core::CallArg {
        name: arg.name.clone(),
        value,
        span: arg.span,
    }
}

fn normalize_brace_lambda_arg(style: core::CallStyle, value: core::Expr) -> core::Expr {
    if style != core::CallStyle::Brace {
        return value;
    }

    let core::Expr::Block { body, span } = value else {
        return value;
    };
    let core::Block {
        statements,
        span: block_span,
    } = body;
    let mut statements = statements;
    if statements.len() != 1 {
        return core::Expr::Block {
            body: core::Block {
                statements,
                span: block_span,
            },
            span,
        };
    }

    match statements.remove(0) {
        core::Stmt::Expr(core::ExprStmt { expr, .. })
            if matches!(&expr, core::Expr::Lambda { .. }) =>
        {
            expr
        }
        stmt => core::Expr::Block {
            body: core::Block {
                statements: vec![stmt],
                span: block_span,
            },
            span,
        },
    }
}

fn desugar_if_condition_clause(clause: &ast::IfConditionClause) -> core::IfConditionClause {
    match clause {
        ast::IfConditionClause::Let(clause) => {
            core::IfConditionClause::Let(desugar_refutable_clause(clause))
        }
        ast::IfConditionClause::Expr(expr) => core::IfConditionClause::Expr(desugar_expr(expr)),
    }
}

fn desugar_refutable_clause(clause: &ast::RefutableClause) -> core::RefutableClause {
    core::RefutableClause {
        pattern: clause.pattern.clone(),
        value: desugar_expr(&clause.value),
        span: clause.span,
    }
}

fn desugar_else_branch(branch: &ast::ElseBranch) -> core::ElseBranch {
    match branch {
        ast::ElseBranch::If(stmt) => core::ElseBranch::If(Box::new(core::IfStmt {
            condition: stmt.condition.as_ref().map(desugar_expr),
            condition_clauses: stmt
                .condition_clauses
                .iter()
                .map(desugar_if_condition_clause)
                .collect(),
            pattern: stmt.pattern.clone(),
            pattern_value: stmt.pattern_value.as_ref().map(desugar_expr),
            pattern_clauses: stmt
                .pattern_clauses
                .iter()
                .map(desugar_refutable_clause)
                .collect(),
            bindings: stmt.bindings.clone(),
            binding_value: stmt.binding_value.as_ref().map(desugar_expr),
            then_block: desugar_block(&stmt.then_block),
            else_branch: stmt.else_branch.as_ref().map(desugar_else_branch),
            span: stmt.span,
        })),
        ast::ElseBranch::Block(block) => core::ElseBranch::Block(desugar_block(block)),
    }
}

fn desugar_else_expr_branch(branch: &ast::ElseExprBranch) -> core::ElseExprBranch {
    match branch {
        ast::ElseExprBranch::If(expr) => core::ElseExprBranch::If(Box::new(desugar_expr(expr))),
        ast::ElseExprBranch::Block(block) => core::ElseExprBranch::Block(desugar_block(block)),
    }
}

fn desugar_match_case(case: &ast::MatchCase) -> core::MatchCase {
    core::MatchCase {
        pattern: case.pattern.clone(),
        guard: case.guard.as_ref().map(desugar_expr),
        body: match &case.body {
            ast::MatchCaseBody::Block(block) => core::MatchCaseBody::Block(desugar_block(block)),
            ast::MatchCaseBody::Expr(expr) => core::MatchCaseBody::Expr(desugar_expr(expr)),
        },
        span: case.span,
    }
}

fn desugar_for_binding(binding: &ast::ForBinding) -> core::ForBinding {
    core::ForBinding {
        bindings: binding.bindings.clone(),
        destructure: binding.destructure,
        pattern: binding.pattern.clone(),
        iterable: binding.iterable.as_ref().map(desugar_expr),
        values: binding.values.iter().map(desugar_expr).collect(),
        span: binding.span,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{SourceFile, lex, parse_program};

    fn parse_function(src: &str) -> ast::FunctionDecl {
        let file = SourceFile::new("test.lum", src);
        let lexed = lex(&file);
        assert!(lexed.diagnostics.is_empty(), "{:#?}", lexed.diagnostics);
        let parsed = parse_program(&lexed.tokens);
        assert!(parsed.diagnostics.is_empty(), "{:#?}", parsed.diagnostics);
        let program = parsed.program.expect("program");
        match &program.items[0] {
            ast::Item::Function(function) => function.clone(),
            other => panic!("expected function, got {other:#?}"),
        }
    }

    #[test]
    fn desugars_trailing_block_lambda_arg_to_plain_lambda() {
        let function = parse_function(
            r#"
def make() Unit = values.map {
    value -> value + 5
}
"#,
        );
        let core = desugar_function_decl(&function);
        match core.body {
            core::CallableBody::Expr(core::Expr::Call {
                args,
                style: core::CallStyle::Brace,
                ..
            }) => {
                assert_eq!(args.len(), 1);
                assert!(matches!(args[0].value, core::Expr::Lambda { .. }));
            }
            other => panic!("expected brace call expression, got {other:#?}"),
        }
    }

    #[test]
    fn removes_group_expr_nodes() {
        let function = parse_function(r#"def make() Int = (1 + 2)"#);
        let core = desugar_function_decl(&function);
        match core.body {
            core::CallableBody::Expr(core::Expr::Binary { .. }) => {}
            other => panic!("expected binary expr after group removal, got {other:#?}"),
        }
    }
}
