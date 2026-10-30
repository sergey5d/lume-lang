use crate::{
    Diagnostic, Keyword, SourceFile, Token, TokenKind,
    ast::{
        Annotation, CallableBody, DeferAction, ElseBranch, ElseExprBranch, Expr, ForBinding,
        FunctionDecl, GenericCondition, IfConditionClause, Item, LambdaBody, MatchCase,
        MatchCaseBody, MethodDecl, Pattern, Program, Stmt, TypeMember, TypeParam, TypeRef,
    },
    lex, parse_program,
    source::Span,
};

const INDENT: &str = "    ";

#[derive(Debug, Clone)]
pub struct FormatResult {
    pub text: String,
    pub diagnostics: Vec<Diagnostic>,
}

impl FormatResult {
    pub fn has_errors(&self) -> bool {
        !self.diagnostics.is_empty()
    }
}

/// Formats one syntactically valid Lume source file.
///
/// This preserves comments, literal spelling, and multiline string contents,
/// while applying the language's canonical callable and type spelling. Invalid
/// input is returned unchanged so callers can safely avoid overwriting the source file.
pub fn format_source(file: &SourceFile) -> FormatResult {
    let lexed = lex(file);
    if lexed.has_errors() {
        return FormatResult {
            text: file.text.clone(),
            diagnostics: lexed.diagnostics,
        };
    }

    let parsed = parse_program(&lexed.tokens);
    if !parsed.diagnostics.is_empty() {
        return FormatResult {
            text: file.text.clone(),
            diagnostics: parsed.diagnostics,
        };
    }

    let program = parsed
        .program
        .expect("a successful parse always produces a program");
    let canonical_source = canonicalize_syntax(&file.text, &lexed.tokens, &program);
    let canonical_file = SourceFile::new(file.name.clone(), canonical_source);
    let canonical_lexed = lex(&canonical_file);
    if canonical_lexed.has_errors() {
        return FormatResult {
            text: file.text.clone(),
            diagnostics: canonical_lexed.diagnostics,
        };
    }

    FormatResult {
        text: format_valid_source(&canonical_file.text, &canonical_lexed.tokens),
        diagnostics: Vec::new(),
    }
}

struct SourceEdit {
    start: usize,
    end: usize,
    replacement: &'static str,
    speculative_type_argument: bool,
}

struct SourceRewrites<'a> {
    source: &'a str,
    edits: Vec<SourceEdit>,
    speculative_type_argument: bool,
    option_value_declared: bool,
}

impl SourceRewrites<'_> {
    fn bind_value_name(&mut self, name: &str) {
        self.option_value_declared |= name == "Option";
    }

    fn push(&mut self, span: Span) {
        self.replace(span.start, span.end, "");
    }

    fn replace(&mut self, start: usize, end: usize, replacement: &'static str) {
        self.edits.push(SourceEdit {
            start,
            end,
            replacement,
            speculative_type_argument: self.speculative_type_argument,
        });
    }
}

fn canonicalize_syntax(source: &str, tokens: &[Token], program: &Program) -> String {
    let mut rewrites = SourceRewrites {
        source,
        edits: Vec::new(),
        speculative_type_argument: false,
        option_value_declared: false,
    };
    collect_program_rewrites(program, tokens, &mut rewrites);
    // Bracket applications can also be indexing. Without name resolution, leave
    // speculative types alone when Option can denote a value in this file.
    if rewrites.option_value_declared {
        rewrites
            .edits
            .retain(|edit| !edit.speculative_type_argument);
    }
    rewrites.edits.sort_by_key(|edit| (edit.start, edit.end));
    rewrites.edits.dedup_by_key(|edit| (edit.start, edit.end));

    let mut result = source.to_string();
    for edit in rewrites.edits.into_iter().rev() {
        result.replace_range(edit.start..edit.end, edit.replacement);
    }
    result
}

fn collect_type_rewrites(ty: &TypeRef, tokens: &[Token], rewrites: &mut SourceRewrites<'_>) {
    match ty {
        TypeRef::Named { name, args, span } => {
            if name == "Option"
                && let [payload] = args.as_slice()
            {
                collect_optional_shorthand(payload, *span, tokens, rewrites);
            }
            for arg in args {
                collect_type_rewrites(arg, tokens, rewrites);
            }
        }
        TypeRef::Tuple { fields, .. } => {
            for field in fields {
                collect_type_rewrites(&field.ty, tokens, rewrites);
            }
        }
        TypeRef::Record { fields, .. } => {
            for field in fields {
                collect_type_rewrites(&field.ty, tokens, rewrites);
            }
        }
        TypeRef::Function { params, ret, .. } => {
            for param in params {
                collect_type_rewrites(param, tokens, rewrites);
            }
            collect_type_rewrites(ret, tokens, rewrites);
        }
        TypeRef::Union { members, .. } => {
            for member in members {
                collect_type_rewrites(member, tokens, rewrites);
            }
        }
        TypeRef::Wildcard { .. } => {}
    }
}

fn collect_optional_shorthand(
    payload: &TypeRef,
    span: Span,
    tokens: &[Token],
    rewrites: &mut SourceRewrites<'_>,
) {
    if matches!(payload, TypeRef::Named { name, args, .. } if name == "Option" && args.len() == 1) {
        return;
    }
    let start = tokens.partition_point(|token| token.span.start < span.start);
    let end = tokens.partition_point(|token| token.span.start < span.end);
    let type_tokens = &tokens[start..end];
    if type_tokens.len() < 4
        || type_tokens[0].lexeme != "Option"
        || type_tokens[1].kind != TokenKind::LBracket
        || type_tokens
            .last()
            .is_none_or(|token| token.kind != TokenKind::RBracket)
        || tokens
            .get(end)
            .is_some_and(|token| token.kind == TokenKind::Question)
    {
        return;
    }
    let payload_tokens = type_tokens[2..type_tokens.len() - 1]
        .iter()
        .filter(|token| token.kind != TokenKind::Newline)
        .collect::<Vec<_>>();
    let Some(first) = payload_tokens.first() else {
        return;
    };
    let last_index = payload_tokens.len()
        - 1
        - usize::from(
            payload_tokens
                .last()
                .is_some_and(|token| token.kind == TokenKind::Comma),
        );
    let last = payload_tokens[last_index];
    let opening = &type_tokens[1];
    let closing = type_tokens.last().unwrap();
    // Keep comments outside the payload in place rather than deleting them.
    if !rewrites.source[type_tokens[0].span.end..opening.span.start]
        .chars()
        .all(char::is_whitespace)
        || !rewrites.source[opening.span.end..first.span.start]
            .chars()
            .all(char::is_whitespace)
        || !rewrites.source[last.span.end..closing.span.start]
            .chars()
            .all(|ch| ch.is_whitespace() || ch == ',')
    {
        return;
    }
    let grouped = first.kind == TokenKind::LParen && first.span.start < payload.span().start;
    let needs_group =
        matches!(payload, TypeRef::Function { .. } | TypeRef::Union { .. }) && !grouped;
    rewrites.replace(
        span.start,
        first.span.start,
        if needs_group { "(" } else { "" },
    );
    rewrites.replace(
        last.span.end,
        span.end,
        if needs_group { ")?" } else { "?" },
    );
}

fn collect_generic_rewrites(
    params: &[TypeParam],
    conditions: &[GenericCondition],
    tokens: &[Token],
    rewrites: &mut SourceRewrites<'_>,
) {
    for param in params {
        for bound in &param.bounds {
            collect_type_rewrites(bound, tokens, rewrites);
        }
    }
    for condition in conditions {
        match condition {
            GenericCondition::Bound { subject, bound, .. } => {
                collect_type_rewrites(subject, tokens, rewrites);
                collect_type_rewrites(bound, tokens, rewrites);
            }
            GenericCondition::Equal { left, right, .. } => {
                collect_type_rewrites(left, tokens, rewrites);
                collect_type_rewrites(right, tokens, rewrites);
            }
        }
    }
}

fn collect_program_rewrites(
    program: &Program,
    tokens: &[Token],
    rewrites: &mut SourceRewrites<'_>,
) {
    for item in &program.items {
        match item {
            Item::Function(function) => {
                collect_redundant_visibility(
                    tokens,
                    function.span.start,
                    Keyword::Private,
                    rewrites,
                );
                collect_function_rewrites(function, tokens, rewrites);
            }
            Item::Type(decl) => {
                if decl.kind == crate::ast::TypeKind::Object {
                    rewrites.bind_value_name(&decl.name);
                }
                collect_generic_rewrites(
                    &decl.type_params,
                    &decl.type_conditions,
                    tokens,
                    rewrites,
                );
                for bound in &decl.with_bounds {
                    collect_type_rewrites(bound, tokens, rewrites);
                }
                collect_redundant_visibility(
                    tokens,
                    decl.span.start,
                    if decl.name.contains('.') {
                        Keyword::Public
                    } else {
                        Keyword::Private
                    },
                    rewrites,
                );
                for annotation in &decl.annotations {
                    collect_annotation_rewrites(annotation, tokens, rewrites);
                }
                for mixin in &decl.mixins {
                    collect_type_rewrites(mixin, tokens, rewrites);
                }
                for member in &decl.members {
                    match member {
                        TypeMember::Field(field) => {
                            rewrites.bind_value_name(&field.name);
                            if let Some(ty) = &field.ty {
                                collect_type_rewrites(ty, tokens, rewrites);
                            }
                            collect_redundant_visibility(
                                tokens,
                                field.span.start,
                                Keyword::Public,
                                rewrites,
                            );
                            for annotation in &field.annotations {
                                collect_annotation_rewrites(annotation, tokens, rewrites);
                            }
                            if let Some(initializer) = &field.initializer {
                                collect_expr_rewrites(initializer, tokens, rewrites);
                            }
                        }
                        TypeMember::Method(method) => {
                            collect_method_rewrites(method, tokens, rewrites)
                        }
                        TypeMember::Case(case) => {
                            for annotation in &case.annotations {
                                collect_annotation_rewrites(annotation, tokens, rewrites);
                            }
                            for field in &case.fields {
                                rewrites.bind_value_name(&field.name);
                                if let Some(ty) = &field.ty {
                                    collect_type_rewrites(ty, tokens, rewrites);
                                }
                                collect_redundant_visibility(
                                    tokens,
                                    field.span.start,
                                    Keyword::Public,
                                    rewrites,
                                );
                                for annotation in &field.annotations {
                                    collect_annotation_rewrites(annotation, tokens, rewrites);
                                }
                                if let Some(initializer) = &field.initializer {
                                    collect_expr_rewrites(initializer, tokens, rewrites);
                                }
                            }
                        }
                    }
                }
            }
            Item::Extension(extension) => {
                collect_type_rewrites(&extension.target, tokens, rewrites);
                for method in &extension.methods {
                    collect_method_rewrites(method, tokens, rewrites);
                }
            }
            Item::Statement(stmt) => {
                if let Stmt::Binding(binding) = stmt {
                    collect_redundant_visibility(
                        tokens,
                        binding.span.start,
                        Keyword::Private,
                        rewrites,
                    );
                }
                collect_stmt_rewrites(stmt, tokens, rewrites);
            }
            Item::TypeAlias(alias) => {
                collect_redundant_visibility(
                    tokens,
                    alias.span.start,
                    if alias.name.contains('.') {
                        Keyword::Public
                    } else {
                        Keyword::Private
                    },
                    rewrites,
                );
                collect_type_rewrites(&alias.target, tokens, rewrites);
                collect_generic_rewrites(
                    &alias.type_params,
                    &alias.type_conditions,
                    tokens,
                    rewrites,
                );
            }
        }
    }
}

fn collect_redundant_visibility(
    tokens: &[Token],
    declaration_start: usize,
    default_keyword: Keyword,
    rewrites: &mut SourceRewrites<'_>,
) {
    if let Some(token) = token_immediately_before(tokens, declaration_start)
        && token.kind == TokenKind::Keyword(default_keyword)
    {
        rewrites.push(token.span);
    }
}

fn collect_annotation_rewrites(
    annotation: &Annotation,
    tokens: &[Token],
    rewrites: &mut SourceRewrites<'_>,
) {
    collect_expr_rewrites(&annotation.value, tokens, rewrites);
}

fn collect_function_rewrites(
    function: &FunctionDecl,
    tokens: &[Token],
    rewrites: &mut SourceRewrites<'_>,
) {
    rewrites.bind_value_name(&function.name);
    collect_generic_rewrites(
        &function.type_params,
        &function.type_conditions,
        tokens,
        rewrites,
    );
    if let Some(ty) = &function.return_type {
        collect_type_rewrites(ty, tokens, rewrites);
    }
    collect_explicit_unit_return(&function.return_type, rewrites);
    for annotation in &function.annotations {
        collect_annotation_rewrites(annotation, tokens, rewrites);
    }
    for param in &function.params {
        rewrites.bind_value_name(&param.name);
        if let Some(ty) = &param.ty {
            collect_type_rewrites(ty, tokens, rewrites);
        }
        if let Some(initializer) = &param.initializer {
            collect_expr_rewrites(initializer, tokens, rewrites);
        }
    }
    collect_callable_body_rewrites(&function.body, function.span, tokens, rewrites);
}

fn collect_method_rewrites(
    method: &MethodDecl,
    tokens: &[Token],
    rewrites: &mut SourceRewrites<'_>,
) {
    rewrites.bind_value_name(&method.name);
    collect_generic_rewrites(
        &method.type_params,
        &method.type_conditions,
        tokens,
        rewrites,
    );
    if let Some(ty) = &method.return_type {
        collect_type_rewrites(ty, tokens, rewrites);
    }
    collect_redundant_visibility(tokens, method.span.start, Keyword::Public, rewrites);
    if !method.getter {
        collect_explicit_unit_return(&method.return_type, rewrites);
    }
    for annotation in &method.annotations {
        collect_annotation_rewrites(annotation, tokens, rewrites);
    }
    for param in &method.params {
        rewrites.bind_value_name(&param.name);
        if let Some(ty) = &param.ty {
            collect_type_rewrites(ty, tokens, rewrites);
        }
        if let Some(initializer) = &param.initializer {
            collect_expr_rewrites(initializer, tokens, rewrites);
        }
    }
    if let Some(body) = &method.body {
        collect_callable_body_rewrites(body, method.span, tokens, rewrites);
    }
}

fn collect_explicit_unit_return(return_type: &Option<TypeRef>, rewrites: &mut SourceRewrites<'_>) {
    if let Some(TypeRef::Named { name, args, span }) = return_type
        && name == "Unit"
        && args.is_empty()
    {
        rewrites.push(*span);
    }
}

fn collect_callable_body_rewrites(
    body: &CallableBody,
    callable_span: Span,
    tokens: &[Token],
    rewrites: &mut SourceRewrites<'_>,
) {
    match body {
        CallableBody::Block(block) => {
            // Assignment-style bodies also use Block internally, without source braces.
            let first = tokens.partition_point(|token| token.span.start < block.span.start);
            if tokens
                .get(first)
                .is_some_and(|token| token.kind == TokenKind::LBrace)
                && let Some(equals) = token_immediately_before(tokens, block.span.start)
                && equals.kind == TokenKind::Eq
                && equals.span.start >= callable_span.start
            {
                rewrites.push(equals.span);
            }
            collect_block_rewrites(block, tokens, rewrites);
        }
        CallableBody::Expr(expr) => collect_expr_rewrites(expr, tokens, rewrites),
    }
}

fn token_immediately_before(tokens: &[Token], offset: usize) -> Option<&Token> {
    tokens
        .iter()
        .take_while(|token| token.span.end <= offset)
        .filter(|token| !matches!(token.kind, TokenKind::Newline | TokenKind::Eof))
        .last()
}

fn collect_block_rewrites(
    block: &crate::ast::Block,
    tokens: &[Token],
    rewrites: &mut SourceRewrites<'_>,
) {
    for statement in &block.statements {
        collect_stmt_rewrites(statement, tokens, rewrites);
    }
}

fn collect_clause_rewrites(
    clause: &crate::ast::RefutableClause,
    tokens: &[Token],
    rewrites: &mut SourceRewrites<'_>,
) {
    collect_pattern_rewrites(&clause.pattern, tokens, rewrites);
    collect_expr_rewrites(&clause.value, tokens, rewrites);
}

fn collect_pattern_rewrites(
    pattern: &Pattern,
    tokens: &[Token],
    rewrites: &mut SourceRewrites<'_>,
) {
    for name in crate::ast::pattern_binding_names(pattern) {
        rewrites.bind_value_name(&name);
    }
    match pattern {
        Pattern::Type { target, .. } => collect_type_rewrites(target, tokens, rewrites),
        Pattern::Extract { inner, .. } | Pattern::Alias { inner, .. } => {
            collect_pattern_rewrites(inner, tokens, rewrites);
        }
        Pattern::Tuple { elements, .. } | Pattern::List { elements, .. } => {
            for element in elements {
                collect_pattern_rewrites(element, tokens, rewrites);
            }
        }
        Pattern::Record { fields, .. } => {
            for field in fields {
                collect_pattern_rewrites(&field.pattern, tokens, rewrites);
            }
        }
        Pattern::Constructor { args, .. } => {
            for arg in args {
                collect_pattern_rewrites(arg, tokens, rewrites);
            }
        }
        Pattern::Literal { value, .. } => collect_expr_rewrites(value, tokens, rewrites),
        Pattern::Wildcard { .. } | Pattern::Binding { .. } => {}
    }
}

fn collect_condition_clause_rewrites(
    clause: &IfConditionClause,
    tokens: &[Token],
    rewrites: &mut SourceRewrites<'_>,
) {
    match clause {
        IfConditionClause::Let(clause) => collect_clause_rewrites(clause, tokens, rewrites),
        IfConditionClause::Expr(expr) => collect_expr_rewrites(expr, tokens, rewrites),
    }
}

fn collect_match_case_rewrites(
    case: &MatchCase,
    tokens: &[Token],
    rewrites: &mut SourceRewrites<'_>,
) {
    collect_pattern_rewrites(&case.pattern, tokens, rewrites);
    if let Some(guard) = &case.guard {
        collect_expr_rewrites(guard, tokens, rewrites);
    }
    match &case.body {
        MatchCaseBody::Block(block) => collect_block_rewrites(block, tokens, rewrites),
        MatchCaseBody::Expr(expr) => collect_expr_rewrites(expr, tokens, rewrites),
    }
}

fn collect_for_binding_rewrites(
    binding: &ForBinding,
    tokens: &[Token],
    rewrites: &mut SourceRewrites<'_>,
) {
    for item in &binding.bindings {
        rewrites.bind_value_name(&item.name);
        if let Some(ty) = &item.ty {
            collect_type_rewrites(ty, tokens, rewrites);
        }
    }
    if let Some(pattern) = &binding.pattern {
        collect_pattern_rewrites(pattern, tokens, rewrites);
    }
    if let Some(iterable) = &binding.iterable {
        collect_expr_rewrites(iterable, tokens, rewrites);
    }
    for value in &binding.values {
        collect_expr_rewrites(value, tokens, rewrites);
    }
}

fn collect_extraction_binding_rewrite(
    stmt: &crate::ast::LetElseStmt,
    tokens: &[Token],
    rewrites: &mut SourceRewrites<'_>,
) {
    if !stmt.clauses.is_empty() {
        return;
    }
    let Pattern::Extract { inner, .. } = &stmt.pattern else {
        return;
    };
    let Pattern::Binding { name, .. } = inner.as_ref() else {
        return;
    };
    let value_span = stmt.value.span();
    let start = tokens.partition_point(|token| token.span.start < stmt.span.start);
    let value_start = tokens.partition_point(|token| token.span.start < value_span.start);
    let head = tokens[start..value_start]
        .iter()
        .filter(|token| token.kind != TokenKind::Newline)
        .collect::<Vec<_>>();
    let [keyword, binding, arrow] = head.as_slice() else {
        return;
    };
    if keyword.kind != TokenKind::Keyword(Keyword::Let)
        || binding.lexeme != *name
        || arrow.kind != TokenKind::LeftArrow
    {
        return;
    }
    let value_end = tokens.partition_point(|token| token.span.start < value_span.end);
    let fallback_start =
        tokens.partition_point(|token| token.span.start < stmt.else_block.span.start);
    let Some(fallback) = tokens.get(fallback_start) else {
        return;
    };
    if fallback.kind != TokenKind::LBrace
        && !matches!(
            stmt.else_block.statements.as_slice(),
            [Stmt::Return(_) | Stmt::Break(_) | Stmt::Continue(_) | Stmt::Expr(_)]
        )
    {
        return;
    }
    let Some(otherwise) = tokens[value_end..fallback_start]
        .iter()
        .find(|token| token.kind == TokenKind::Keyword(Keyword::Else))
    else {
        return;
    };
    let needs_group = matches!(
        stmt.value,
        Expr::ExtractOr { .. }
            | Expr::If { .. }
            | Expr::ForYield { .. }
            | Expr::Lambda { .. }
            | Expr::Binary { .. }
            | Expr::Unary { .. }
            | Expr::Try { .. }
            | Expr::Return { .. }
    );
    rewrites.push(keyword.span);
    rewrites.replace(arrow.span.start, arrow.span.end, "=");
    rewrites.push(otherwise.span);
    if needs_group {
        rewrites.replace(value_span.start, value_span.start, "(");
    }
    // Attach ?? before any trailing comments/newlines, which may precede else.
    rewrites.replace(
        value_span.end,
        value_span.end,
        if needs_group { ") ??" } else { " ??" },
    );
}

fn collect_stmt_rewrites(stmt: &Stmt, tokens: &[Token], rewrites: &mut SourceRewrites<'_>) {
    match stmt {
        Stmt::Binding(stmt) => {
            for binding in &stmt.bindings {
                rewrites.bind_value_name(&binding.name);
                if let Some(ty) = &binding.ty {
                    collect_type_rewrites(ty, tokens, rewrites);
                }
            }
            for value in &stmt.values {
                collect_expr_rewrites(value, tokens, rewrites);
            }
        }
        Stmt::PatternBinding(stmt) => {
            collect_pattern_rewrites(&stmt.pattern, tokens, rewrites);
            for clause in &stmt.clauses {
                collect_clause_rewrites(clause, tokens, rewrites);
            }
            collect_expr_rewrites(&stmt.value, tokens, rewrites);
        }
        Stmt::Assignment(stmt) => {
            for target in &stmt.targets {
                collect_expr_rewrites(target, tokens, rewrites);
            }
            for value in &stmt.values {
                collect_expr_rewrites(value, tokens, rewrites);
            }
        }
        Stmt::Defer(stmt) => match &stmt.action {
            DeferAction::Call(expr) => collect_expr_rewrites(expr, tokens, rewrites),
            DeferAction::Block(block) => collect_block_rewrites(block, tokens, rewrites),
        },
        Stmt::If(stmt) => {
            if let Some(pattern) = &stmt.pattern {
                collect_pattern_rewrites(pattern, tokens, rewrites);
            }
            for binding in &stmt.bindings {
                rewrites.bind_value_name(&binding.name);
                if let Some(ty) = &binding.ty {
                    collect_type_rewrites(ty, tokens, rewrites);
                }
            }
            if let Some(condition) = &stmt.condition {
                collect_expr_rewrites(condition, tokens, rewrites);
            }
            for clause in &stmt.condition_clauses {
                collect_condition_clause_rewrites(clause, tokens, rewrites);
            }
            if let Some(value) = &stmt.pattern_value {
                collect_expr_rewrites(value, tokens, rewrites);
            }
            for clause in &stmt.pattern_clauses {
                collect_clause_rewrites(clause, tokens, rewrites);
            }
            if let Some(value) = &stmt.binding_value {
                collect_expr_rewrites(value, tokens, rewrites);
            }
            collect_block_rewrites(&stmt.then_block, tokens, rewrites);
            if let Some(branch) = &stmt.else_branch {
                match branch {
                    ElseBranch::If(stmt) => {
                        collect_stmt_rewrites(&Stmt::If((**stmt).clone()), tokens, rewrites)
                    }
                    ElseBranch::Block(block) => collect_block_rewrites(block, tokens, rewrites),
                }
            }
        }
        Stmt::Match(stmt) => {
            collect_expr_rewrites(&stmt.value, tokens, rewrites);
            for case in &stmt.cases {
                collect_match_case_rewrites(case, tokens, rewrites);
            }
        }
        Stmt::While(stmt) => {
            for clause in &stmt.condition_clauses {
                collect_condition_clause_rewrites(clause, tokens, rewrites);
            }
            collect_block_rewrites(&stmt.body, tokens, rewrites);
        }
        Stmt::For(stmt) => {
            for binding in &stmt.bindings {
                collect_for_binding_rewrites(binding, tokens, rewrites);
            }
            collect_block_rewrites(&stmt.body, tokens, rewrites);
        }
        Stmt::LetElse(stmt) => {
            collect_extraction_binding_rewrite(stmt, tokens, rewrites);
            collect_pattern_rewrites(&stmt.pattern, tokens, rewrites);
            for clause in &stmt.clauses {
                collect_clause_rewrites(clause, tokens, rewrites);
            }
            collect_expr_rewrites(&stmt.value, tokens, rewrites);
            collect_block_rewrites(&stmt.else_block, tokens, rewrites);
        }
        Stmt::Return(stmt) => {
            if let Some(value) = &stmt.value {
                collect_expr_rewrites(value, tokens, rewrites);
            }
        }
        Stmt::Expr(stmt) => collect_expr_rewrites(&stmt.expr, tokens, rewrites),
        Stmt::LocalFunction(function) => collect_function_rewrites(function, tokens, rewrites),
        Stmt::Break(_) | Stmt::Continue(_) => {}
    }
}

fn collect_expr_rewrites(expr: &Expr, tokens: &[Token], rewrites: &mut SourceRewrites<'_>) {
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
        Expr::TypeOf { ty, .. } => collect_type_rewrites(ty, tokens, rewrites),
        Expr::Spread { value, .. } | Expr::Try { value, .. } => {
            collect_expr_rewrites(value, tokens, rewrites)
        }
        Expr::ListLiteral { items, .. } | Expr::TupleLiteral { items, .. } => {
            for item in items {
                collect_expr_rewrites(item, tokens, rewrites);
            }
        }
        Expr::Call { callee, args, .. } => {
            collect_expr_rewrites(callee, tokens, rewrites);
            for arg in args {
                if let Some(ty) = &arg.ty {
                    collect_type_rewrites(ty, tokens, rewrites);
                }
                collect_expr_rewrites(&arg.value, tokens, rewrites);
            }
        }
        Expr::ContextualNew { args, .. } => {
            for arg in args {
                if let Some(ty) = &arg.ty {
                    collect_type_rewrites(ty, tokens, rewrites);
                }
                collect_expr_rewrites(&arg.value, tokens, rewrites);
            }
        }
        Expr::Member { receiver, .. } => collect_expr_rewrites(receiver, tokens, rewrites),
        Expr::Index {
            receiver,
            index,
            explicit_type_args,
            ..
        } => {
            collect_expr_rewrites(receiver, tokens, rewrites);
            collect_expr_rewrites(index, tokens, rewrites);
            if let Some(args) = explicit_type_args {
                let previous = rewrites.speculative_type_argument;
                rewrites.speculative_type_argument |= !matches!(index.as_ref(), Expr::Unit { .. });
                for arg in args {
                    collect_type_rewrites(arg, tokens, rewrites);
                }
                rewrites.speculative_type_argument = previous;
            }
        }
        Expr::RecordLiteral { fields, values, .. } => {
            for field in fields {
                if let Some(ty) = &field.ty {
                    collect_type_rewrites(ty, tokens, rewrites);
                }
                collect_expr_rewrites(&field.value, tokens, rewrites);
            }
            for value in values {
                collect_expr_rewrites(value, tokens, rewrites);
            }
        }
        Expr::AnonymousObject {
            fields,
            methods,
            interfaces,
            ..
        } => {
            for interface in interfaces {
                collect_type_rewrites(interface, tokens, rewrites);
            }
            for field in fields {
                rewrites.bind_value_name(&field.name);
                if let Some(ty) = &field.ty {
                    collect_type_rewrites(ty, tokens, rewrites);
                }
                for annotation in &field.annotations {
                    collect_annotation_rewrites(annotation, tokens, rewrites);
                }
                if let Some(initializer) = &field.initializer {
                    collect_expr_rewrites(initializer, tokens, rewrites);
                }
            }
            for method in methods {
                collect_method_rewrites(method, tokens, rewrites);
            }
        }
        Expr::ExtractOr {
            value, fallback, ..
        } => {
            collect_expr_rewrites(value, tokens, rewrites);
            collect_expr_rewrites(fallback, tokens, rewrites);
        }
        Expr::Return { value, .. } => {
            if let Some(value) = value {
                collect_expr_rewrites(value, tokens, rewrites);
            }
        }
        Expr::Unary { expr, .. } | Expr::Group { inner: expr, .. } => {
            collect_expr_rewrites(expr, tokens, rewrites)
        }
        Expr::Binary { left, right, .. } => {
            collect_expr_rewrites(left, tokens, rewrites);
            collect_expr_rewrites(right, tokens, rewrites);
        }
        Expr::Is { left, target, .. } => {
            collect_expr_rewrites(left, tokens, rewrites);
            collect_type_rewrites(target, tokens, rewrites);
        }
        Expr::If {
            condition_clauses,
            then_block,
            else_branch,
            ..
        } => {
            for clause in condition_clauses {
                collect_condition_clause_rewrites(clause, tokens, rewrites);
            }
            collect_block_rewrites(then_block, tokens, rewrites);
            match else_branch.as_ref() {
                ElseExprBranch::If(expr) => collect_expr_rewrites(expr, tokens, rewrites),
                ElseExprBranch::Block(block) => collect_block_rewrites(block, tokens, rewrites),
            }
        }
        Expr::Block { body, .. } => collect_block_rewrites(body, tokens, rewrites),
        Expr::Match { value, cases, .. } => {
            collect_expr_rewrites(value, tokens, rewrites);
            for case in cases {
                collect_match_case_rewrites(case, tokens, rewrites);
            }
        }
        Expr::ForYield {
            bindings,
            yield_body,
            ..
        } => {
            for binding in bindings {
                collect_for_binding_rewrites(binding, tokens, rewrites);
            }
            collect_block_rewrites(yield_body, tokens, rewrites);
        }
        Expr::Lambda { params, body, .. } => {
            for param in params {
                rewrites.bind_value_name(&param.name);
                if let Some(ty) = &param.ty {
                    collect_type_rewrites(ty, tokens, rewrites);
                }
                if let Some(destructure) = &param.destructure {
                    for binding in &destructure.bindings {
                        rewrites.bind_value_name(&binding.name);
                        if let Some(ty) = &binding.ty {
                            collect_type_rewrites(ty, tokens, rewrites);
                        }
                    }
                }
            }
            match body {
                LambdaBody::Expr(expr) => collect_expr_rewrites(expr, tokens, rewrites),
                LambdaBody::Block(block) => collect_block_rewrites(block, tokens, rewrites),
            }
        }
    }
}

#[derive(Clone, Copy)]
struct SourceLine<'a> {
    start: usize,
    content: &'a str,
    ending: &'a str,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DelimiterKind {
    Parenthesis,
    Bracket,
}

#[derive(Clone, Copy)]
struct OpenDelimiter {
    kind: DelimiterKind,
    opened_line: usize,
    suspended_by_brace_depth: Option<usize>,
    carries_continuation: bool,
}

fn format_valid_source(source: &str, tokens: &[Token]) -> String {
    let lines = source_lines(source);
    if lines.is_empty() {
        return String::new();
    }

    let mut tokens_by_line = vec![Vec::new(); lines.len()];
    let mut protected_lines = vec![false; lines.len()];
    for token in tokens {
        if matches!(token.kind, TokenKind::Newline | TokenKind::Eof) {
            continue;
        }

        let line_index = token.span.start_pos.line.saturating_sub(1);
        if let Some(line_tokens) = tokens_by_line.get_mut(line_index) {
            line_tokens.push(token);
        }

        if token.kind == TokenKind::String && token.span.end_pos.line > token.span.start_pos.line {
            let start = token.span.start_pos.line.saturating_sub(1);
            let end = token.span.end_pos.line.min(lines.len());
            for protected in &mut protected_lines[start..end] {
                *protected = true;
            }
        }
    }

    let final_newline = lines
        .iter()
        .find_map(|line| (!line.ending.is_empty()).then_some(line.ending))
        .unwrap_or("\n");
    let mut output = String::with_capacity(source.len() + INDENT.len());
    let mut brace_depth = 0usize;
    let mut brace_continuations = Vec::new();
    let mut delimiters = Vec::new();
    let mut continuation_pending = false;

    for (index, line) in lines.iter().enumerate() {
        let line_tokens = &tokens_by_line[index];

        if protected_lines[index] {
            output.push_str(line.content);
            output.push_str(line.ending);
            update_nesting(
                line_tokens,
                index,
                false,
                &mut brace_depth,
                &mut brace_continuations,
                &mut delimiters,
            );
            continuation_pending = line_requests_continuation(line_tokens);
            continue;
        }

        let without_indent = line.content.trim_start_matches([' ', '\t']);
        let body = without_indent.trim_end_matches([' ', '\t']);
        if body.is_empty() {
            output.push_str(line.ending);
            continue;
        }
        let body_start = line.start + line.content.len() - without_indent.len();
        let body_end = body_start + body.len();
        let body = normalize_horizontal_spacing(source, body_start, body_end, line_tokens);

        let (line_brace_depth, line_delimiters) =
            nesting_after_leading_closers(line_tokens, brace_depth, &delimiters);
        let has_active_delimiter = line_delimiters.iter().any(|delimiter| {
            delimiter
                .suspended_by_brace_depth
                .is_none_or(|depth| brace_depth < depth)
        });
        let starts_continuation = line_tokens
            .first()
            .is_some_and(|token| starts_with_continuation(token.kind));
        let starts_with_closer = line_tokens.first().is_some_and(|token| {
            matches!(
                token.kind,
                TokenKind::RBrace | TokenKind::RParen | TokenKind::RBracket
            )
        });
        let continuation = has_active_delimiter
            || (!starts_with_closer && (continuation_pending || starts_continuation));
        let carries_delimiter_continuation = delimiters.iter().any(|delimiter| {
            delimiter.carries_continuation
                && delimiter
                    .suspended_by_brace_depth
                    .is_none_or(|depth| brace_depth < depth)
        });

        let carried_continuations = brace_continuations
            .iter()
            .filter(|carried| **carried)
            .count();
        for _ in 0..line_brace_depth
            + carried_continuations
            + usize::from(carries_delimiter_continuation)
            + usize::from(continuation)
        {
            output.push_str(INDENT);
        }
        output.push_str(&body);
        output.push_str(line.ending);

        update_nesting(
            line_tokens,
            index,
            continuation,
            &mut brace_depth,
            &mut brace_continuations,
            &mut delimiters,
        );
        if !line_tokens.is_empty() {
            continuation_pending = line_requests_continuation(line_tokens);
        }
    }

    if !output.is_empty() && !output.ends_with(['\n', '\r']) {
        output.push_str(final_newline);
    }
    output
}

fn source_lines(source: &str) -> Vec<SourceLine<'_>> {
    let mut lines = Vec::new();
    let mut start = 0usize;
    for (newline, _) in source.match_indices('\n') {
        let (content_end, ending_start) =
            if newline > start && source.as_bytes()[newline - 1] == b'\r' {
                (newline - 1, newline - 1)
            } else {
                (newline, newline)
            };
        lines.push(SourceLine {
            start,
            content: &source[start..content_end],
            ending: &source[ending_start..=newline],
        });
        start = newline + 1;
    }
    if start < source.len() {
        lines.push(SourceLine {
            start,
            content: &source[start..],
            ending: "",
        });
    }
    lines
}

fn normalize_horizontal_spacing(
    source: &str,
    start: usize,
    end: usize,
    tokens: &[&Token],
) -> String {
    let mut output = String::with_capacity(end.saturating_sub(start));
    let mut cursor = start;
    let mut previous_token = None;

    for (index, token) in tokens.iter().enumerate() {
        if token.span.start < start || token.span.end > end {
            continue;
        }

        let gap = &source[cursor..token.span.start];
        if gap.chars().all(|ch| matches!(ch, ' ' | '\t')) {
            if should_write_horizontal_space(previous_token, token, tokens, index, !gap.is_empty())
            {
                output.push(' ');
            }
        } else {
            output.push_str(gap);
        }
        output.push_str(&source[token.span.start..token.span.end]);
        cursor = token.span.end;
        previous_token = Some(*token);
    }

    output.push_str(&source[cursor..end]);
    output
}

fn should_write_horizontal_space(
    previous: Option<&Token>,
    current: &Token,
    tokens: &[&Token],
    current_index: usize,
    had_space: bool,
) -> bool {
    let Some(previous) = previous else {
        return false;
    };

    if tokens
        .first()
        .is_some_and(|token| token.kind == TokenKind::Keyword(Keyword::Use))
        && (previous.kind == TokenKind::Slash || current.kind == TokenKind::Slash)
    {
        return false;
    }
    if is_selective_import_line(tokens)
        && (previous.kind == TokenKind::LBrace || current.kind == TokenKind::RBrace)
    {
        return false;
    }

    if matches!(
        current.kind,
        TokenKind::RParen
            | TokenKind::RBracket
            | TokenKind::Comma
            | TokenKind::Dot
            | TokenKind::Colon
    ) || matches!(
        previous.kind,
        TokenKind::LParen | TokenKind::LBracket | TokenKind::Dot | TokenKind::Caret
    ) {
        return false;
    }

    if current.kind == TokenKind::LParen && previous.kind == TokenKind::Keyword(Keyword::Fn) {
        return false;
    }

    if current.kind == TokenKind::LParen && can_end_callee(previous.kind) {
        return is_parenthesized_getter_return(tokens, current_index);
    }

    if current.kind == TokenKind::LBracket {
        return had_space;
    }

    had_space || requires_horizontal_space(previous.kind, current.kind)
}

fn is_selective_import_line(tokens: &[&Token]) -> bool {
    tokens
        .first()
        .is_some_and(|token| token.kind == TokenKind::Keyword(Keyword::Use))
        && tokens
            .windows(2)
            .any(|pair| pair[0].kind == TokenKind::Slash && pair[1].kind == TokenKind::LBrace)
}

fn requires_horizontal_space(previous: TokenKind, current: TokenKind) -> bool {
    matches!(previous, TokenKind::LBrace) && current != TokenKind::RBrace
        || matches!(current, TokenKind::LBrace) && previous != TokenKind::LBrace
        || matches!(current, TokenKind::RBrace) && previous != TokenKind::LBrace
        || matches!(previous, TokenKind::Comma | TokenKind::Colon)
        || horizontal_operator(previous)
        || horizontal_operator(current)
}

fn horizontal_operator(kind: TokenKind) -> bool {
    matches!(
        kind,
        TokenKind::Eq
            | TokenKind::FatArrow
            | TokenKind::LeftArrow
            | TokenKind::QuestionQuestion
            | TokenKind::ColonAssign
            | TokenKind::EqEq
            | TokenKind::NotEq
            | TokenKind::Less
            | TokenKind::LessEq
            | TokenKind::Greater
            | TokenKind::GreaterEq
            | TokenKind::PlusEq
            | TokenKind::MinusEq
            | TokenKind::StarEq
            | TokenKind::SlashEq
            | TokenKind::PercentEq
            | TokenKind::AndAnd
            | TokenKind::OrOr
            | TokenKind::Pipe
    )
}

fn can_end_callee(kind: TokenKind) -> bool {
    matches!(
        kind,
        TokenKind::Identifier | TokenKind::RParen | TokenKind::RBracket | TokenKind::Bang
    )
}

fn is_parenthesized_getter_return(tokens: &[&Token], lparen_index: usize) -> bool {
    lparen_index >= 2
        && tokens[lparen_index - 1].kind == TokenKind::Identifier
        && tokens[lparen_index - 2].kind == TokenKind::Keyword(Keyword::Get)
}

fn nesting_after_leading_closers(
    tokens: &[&Token],
    mut braces: usize,
    delimiters: &[OpenDelimiter],
) -> (usize, Vec<OpenDelimiter>) {
    let mut delimiters = delimiters.to_vec();
    for token in tokens {
        match token.kind {
            TokenKind::RBrace => braces = braces.saturating_sub(1),
            TokenKind::RParen => pop_delimiter(&mut delimiters, DelimiterKind::Parenthesis),
            TokenKind::RBracket => pop_delimiter(&mut delimiters, DelimiterKind::Bracket),
            _ => break,
        }
    }
    (braces, delimiters)
}

fn update_nesting(
    tokens: &[&Token],
    line: usize,
    carry_continuation: bool,
    braces: &mut usize,
    brace_continuations: &mut Vec<bool>,
    delimiters: &mut Vec<OpenDelimiter>,
) {
    let mut carry_continuation = carry_continuation;
    for (index, token) in tokens.iter().enumerate() {
        match token.kind {
            TokenKind::LBrace => {
                let closes_on_line = matching_brace_closes_on_line(tokens, index);
                *braces += 1;
                brace_continuations.push(carry_continuation && !closes_on_line);
                if !closes_on_line {
                    carry_continuation = false;
                    for delimiter in delimiters.iter_mut().rev() {
                        if delimiter.opened_line != line {
                            break;
                        }
                        delimiter.suspended_by_brace_depth.get_or_insert(*braces);
                    }
                }
            }
            TokenKind::RBrace => {
                *braces = braces.saturating_sub(1);
                brace_continuations.pop();
            }
            TokenKind::LParen => delimiters.push(OpenDelimiter {
                kind: DelimiterKind::Parenthesis,
                opened_line: line,
                suspended_by_brace_depth: None,
                carries_continuation: carry_continuation,
            }),
            TokenKind::RParen => pop_delimiter(delimiters, DelimiterKind::Parenthesis),
            TokenKind::LBracket => delimiters.push(OpenDelimiter {
                kind: DelimiterKind::Bracket,
                opened_line: line,
                suspended_by_brace_depth: None,
                carries_continuation: carry_continuation,
            }),
            TokenKind::RBracket => pop_delimiter(delimiters, DelimiterKind::Bracket),
            _ => {}
        }
    }
}

fn matching_brace_closes_on_line(tokens: &[&Token], opening: usize) -> bool {
    let mut depth = 0usize;
    for token in &tokens[opening..] {
        match token.kind {
            TokenKind::LBrace => depth += 1,
            TokenKind::RBrace => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return true;
                }
            }
            _ => {}
        }
    }
    false
}

fn pop_delimiter(delimiters: &mut Vec<OpenDelimiter>, kind: DelimiterKind) {
    if delimiters
        .last()
        .is_some_and(|delimiter| delimiter.kind == kind)
    {
        delimiters.pop();
    }
}

fn line_requests_continuation(tokens: &[&Token]) -> bool {
    let Some(last) = tokens.last() else {
        return false;
    };
    if tokens
        .first()
        .is_some_and(|token| token.kind == TokenKind::Keyword(crate::Keyword::Use))
        && last.kind == TokenKind::Star
    {
        return false;
    }
    ends_with_continuation(last.kind) && !ends_with_braced_lambda_head(tokens)
}

fn ends_with_braced_lambda_head(tokens: &[&Token]) -> bool {
    if !tokens
        .last()
        .is_some_and(|token| token.kind == TokenKind::FatArrow)
    {
        return false;
    }
    let Some(brace) = tokens
        .iter()
        .rposition(|token| token.kind == TokenKind::LBrace)
    else {
        return false;
    };
    !tokens[brace + 1..]
        .iter()
        .any(|token| token.kind == TokenKind::Keyword(crate::Keyword::Case))
}

fn starts_with_continuation(kind: TokenKind) -> bool {
    matches!(kind, TokenKind::Dot | TokenKind::Pipe)
}

fn ends_with_continuation(kind: TokenKind) -> bool {
    matches!(
        kind,
        TokenKind::Dot
            | TokenKind::Colon
            | TokenKind::Plus
            | TokenKind::Minus
            | TokenKind::Star
            | TokenKind::Slash
            | TokenKind::Percent
            | TokenKind::Eq
            | TokenKind::FatArrow
            | TokenKind::LeftArrow
            | TokenKind::QuestionQuestion
            | TokenKind::ColonAssign
            | TokenKind::EqEq
            | TokenKind::NotEq
            | TokenKind::Less
            | TokenKind::LessEq
            | TokenKind::Greater
            | TokenKind::GreaterEq
            | TokenKind::PlusEq
            | TokenKind::MinusEq
            | TokenKind::StarEq
            | TokenKind::SlashEq
            | TokenKind::PercentEq
            | TokenKind::AndAnd
            | TokenKind::OrOr
            | TokenKind::Pipe
            | TokenKind::Keyword(crate::Keyword::As)
            | TokenKind::Keyword(crate::Keyword::Do)
            | TokenKind::Keyword(crate::Keyword::Else)
            | TokenKind::Keyword(crate::Keyword::Is)
            | TokenKind::Keyword(crate::Keyword::When)
            | TokenKind::Keyword(crate::Keyword::With)
            | TokenKind::Keyword(crate::Keyword::Yield)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn format(text: &str) -> FormatResult {
        format_source(&SourceFile::new("test.lum", text))
    }

    fn assert_formatted(source: &str, expected: &str) {
        let result = format(source);
        assert!(!result.has_errors(), "{:#?}", result.diagnostics);
        assert_eq!(result.text, expected);
        let file = SourceFile::new("formatted.lum", result.text.clone());
        let lexed = lex(&file);
        assert!(!lexed.has_errors(), "{:#?}", lexed.diagnostics);
        let parsed = parse_program(&lexed.tokens);
        assert!(parsed.diagnostics.is_empty(), "{:#?}", parsed.diagnostics);
        let second = format(&result.text);
        assert!(!second.has_errors(), "{:#?}", second.diagnostics);
        assert_eq!(second.text, result.text);
    }

    #[test]
    fn canonicalizes_option_type_annotations() {
        assert_formatted(
            "type MaybeId = Option[Int]\nshape Envelope {\nvalue Option[Str]\nget optionalValue Option[Str] = value\nfn echo(input Option[Int]) Option[Int] = input\n}\nfn keep[T](value T) T = value\nfn main() {\nvalues [Option[Int]] = [None]\nlocal Option[Int] = keep[Option[Int]](values[0])\nreader fn(Option[Int]) Option[Str] =\n(item Option[Int]) => None\nprintln(typeOf[Option[Int]])\n}\n",
            "type MaybeId = Int?\nshape Envelope {\n    value Str?\n    get optionalValue Str? = value\n    fn echo(input Int?) Int? = input\n}\nfn keep[T](value T) T = value\nfn main() {\n    values [Int?] = [None]\n    local Int? = keep[Int?](values[0])\n    reader fn(Int?) Str? =\n        (item Int?) => None\n    println(typeOf[Int?])\n}\n",
        );
    }

    #[test]
    fn canonicalizes_simple_extraction_bindings_to_extract_or() {
        assert_formatted(
            "fn parse(minutesText Str) Int? {\nlet minutes <- Int.parse(minutesText) else return None\nSome(minutes)\n}\nfn fallback(value Int?) Int {\nlet item <- value else 42\nitem\n}\nfn main() {\nfor value <- [Some(1), None] {\nlet item <- value else continue\nlet next <- value else break\nprintln(item, next)\n}\n}\n",
            "fn parse(minutesText Str) Int? {\n    minutes = Int.parse(minutesText) ?? return None\n    Some(minutes)\n}\nfn fallback(value Int?) Int {\n    item = value ?? 42\n    item\n}\nfn main() {\n    for value <- [Some(1), None] {\n        item = value ?? continue\n        next = value ?? break\n        println(item, next)\n    }\n}\n",
        );
    }

    #[test]
    fn extraction_rewrites_preserve_comments_and_fallback_blocks() {
        assert_formatted(
            "fn parse(text Str) Int? {\nlet minutes <-\nInt.parse(text) # source comment\n# fallback comment\nelse return None # exit comment\nlet next <- Int.parse(text) else {\nprintln(\"missing\")\nreturn None\n}\nSome(minutes + next)\n}\n",
            "fn parse(text Str) Int? {\n    minutes =\n        Int.parse(text) ?? # source comment\n        # fallback comment\n        return None # exit comment\n    next = Int.parse(text) ?? {\n        println(\"missing\")\n        return None\n    }\n    Some(minutes + next)\n}\n",
        );
    }

    #[test]
    fn extraction_rewrites_keep_the_complete_source_expression() {
        assert_formatted(
            "fn nested(left Option[Int?], right Int?, ready Bool) Int {\nlet first <- left ?? right else return -1\nlet second <- if ready { right } else right else 2\nlet third <- for value <- right yield value + 1 else 3\nfirst + second + third\n}\n",
            "fn nested(left Option[Int?], right Int?, ready Bool) Int {\n    first = (left ?? right) ?? return -1\n    second = (if ready { right } else right) ?? 2\n    third = (for value <- right yield value + 1) ?? 3\n    first + second + third\n}\n",
        );
    }

    #[test]
    fn keeps_destructuring_typed_and_grouped_extractions_as_patterns() {
        assert_formatted(
            "type Pair = (Int, Int)\nfn patterns(value Int?, pair Pair?, left Int?, right Int?) {\nlet item Int <- value else return\nlet Some { value as item } = left else return\nlet (a, b) <- pair else return\nlet _ <- left else return\nlet {\nx <- left\ny <- right\n} else return\n}\n",
            "type Pair = (Int, Int)\nfn patterns(value Int?, pair Pair?, left Int?, right Int?) {\n    let item Int <- value else return\n    let Some { value as item } = left else return\n    let (a, b) <- pair else return\n    let _ <- left else return\n    let {\n        x <- left\n        y <- right\n    } else return\n}\n",
        );
    }

    #[test]
    fn rejects_parenthesized_unpacking_without_overwriting_source() {
        let source = "fn run(result Result[Int, Str]) {\nlet Ok(value) = result else return\nprintln(value)\n}\n";
        let result = format(source);
        assert_eq!(result.text, source);
        assert!(
            result
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "parenthesized_pattern")
        );
        assert_formatted(
            "fn run(result Result[Int, Str]) {\nlet Ok { value } = result else return\nprintln(value)\n}\n",
            "fn run(result Result[Int, Str]) {\n    let Ok { value } = result else return\n    println(value)\n}\n",
        );
    }

    #[test]
    fn extraction_rewrites_preserve_runtime_behavior() {
        let source = r#"
fn early(value Int?) Int {
    let item <- value else return -1
    item
}
fn resultValue(value Result[Int, Str]) Int {
    let item <- value else 42
    item
}
fn eitherValue(value Either[Str, Int]) Int {
    let item <- value else 0
    item
}
fn nested(left Option[Int?], right Int?) Int {
    let item <- left ?? right else return -1
    item
}
fn conditional(ready Bool, left Int?, right Int?) Int {
    let item <- if ready { left } else right else 7
    item
}
fn yielded(value Int?) Int {
    let item <- for number <- value yield number + 1 else 0
    item
}
fn loops(values [Int?]) Int {
    var total = 0
    for value <- values {
        let item <- value else continue
        total += item
    }
    for value <- values {
        let item <- value else break
        total += item
    }
    total
}
fn main() {
    println(
        early(None), early(Some(5)), resultValue(Err("missing")),
        eitherValue(Right(3)), nested(Some(None), Some(9)),
        conditional(true, None, Some(9)), yielded(Some(2)),
        loops([Some(1), None, Some(3)])
    )
}
"#;
        fn run(source: &str) -> String {
            let file = SourceFile::new("test.lum", source);
            let lexed = lex(&file);
            assert!(!lexed.has_errors(), "{:#?}", lexed.diagnostics);
            let parsed = parse_program(&lexed.tokens);
            assert!(parsed.diagnostics.is_empty(), "{:#?}", parsed.diagnostics);
            let program = parsed.program.unwrap();
            let checked = crate::check_program(&program);
            assert!(checked.diagnostics.is_empty(), "{:#?}", checked.diagnostics);
            let lowered = crate::lower_program(&program);
            assert!(lowered.diagnostics.is_empty(), "{:#?}", lowered.diagnostics);
            let result = crate::run_program(&lowered.program.unwrap());
            assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
            result.output
        }
        let formatted = format(source);
        assert!(!formatted.has_errors(), "{:#?}", formatted.diagnostics);
        let expected = "-1 5 42 3 -1 7 3 5\n";
        assert_eq!(run(source), expected);
        assert_eq!(run(&formatted.text), expected);
    }

    #[test]
    fn optional_shorthand_preserves_type_grouping_and_nesting() {
        for (source, expected) in [
            ("Option[fn(Int) Str]", "(fn(Int) Str)?"),
            ("Option[fn(Option[Int]) Option[Str]]", "(fn(Int?) Str?)?"),
            ("Option[(fn() Int)]", "(fn() Int)?"),
            ("Option[Int | Str]", "(Int | Str)?"),
            ("Option[(Int | Str)]", "(Int | Str)?"),
            ("Option[(Option[Int], Option[Str])]", "(Int?, Str?)?"),
            ("Option[[Option[Int]]]", "[Int?]?"),
            ("Option[{ value Option[Int] }]", "{ value Int? }?"),
            ("Option[Option[Int]]", "Option[Int?]"),
            ("Option[Option[Option[Int]]]", "Option[Option[Int?]]"),
            ("Option[Int]?", "Option[Int]?"),
            ("Option[Int?]", "Option[Int?]"),
            ("Option[Unit]", "Unit?"),
            ("Option[Int,]", "Int?"),
            ("models.Option[Int]", "models.Option[Int]"),
        ] {
            assert_formatted(
                &format!("type Value = {source}"),
                &format!("type Value = {expected}\n"),
            );
        }
    }

    #[test]
    fn optional_shorthand_preserves_comments_and_literal_text() {
        assert_formatted(
            "# Option[Int] stays in this comment\ntype Multiline = Option[\nInt,\n]\ntype Commented = Option[\n# payload comment\nInt,\n]\nfn text() Str = \"Option[Int]\"\n",
            "# Option[Int] stays in this comment\ntype Multiline = Int?\ntype Commented = Option[\n    # payload comment\n    Int,\n]\nfn text() Str = \"Option[Int]\"\n",
        );
    }

    #[test]
    fn optional_shorthand_does_not_rewrite_value_indexing() {
        assert_formatted(
            "fn main() {\nOption = [0]\nInt = 0\nhandlers = [() => 5]\nvalue Option[Str] = None\nprintln(handlers[Option[Int]]())\n}\n",
            "fn main() {\n    Option = [0]\n    Int = 0\n    handlers = [() => 5]\n    value Str? = None\n    println(handlers[Option[Int]]())\n}\n",
        );
    }

    #[test]
    fn formats_blocks_continuations_and_trailing_whitespace() {
        let result = format(
            "fn main() Unit {\nprintln(\"start\")  \nvalue = 1 +\n2\nif true {\nprintln(value)\n}\n}\n",
        );
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
        assert_eq!(
            result.text,
            "fn main() {\n    println(\"start\")\n    value = 1 +\n        2\n    if true {\n        println(value)\n    }\n}\n"
        );
    }

    #[test]
    fn canonicalizes_spaces_before_callable_parameter_lists() {
        let result = format(
            "fn calculate (input Int) Int = input\nfn identity[T] (input T) T = input\nclass Calculator {\nfn calculate (input Int) Int = input\nfn convert[T] () T = panic(\"missing\")\n}\n",
        );
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
        assert_eq!(
            result.text,
            "fn calculate(input Int) Int = input\nfn identity[T](input T) T = input\nclass Calculator {\n    fn calculate(input Int) Int = input\n    fn convert[T]() T = panic(\"missing\")\n}\n"
        );
    }

    #[test]
    fn formats_fn_declarations_and_function_types() {
        assert_formatted(
            "interface Worker {\nfn start()\nfn status() Str\nfn callback() fn () Str\n}\nfn normalize (value Str) Str=value.trim()\nfn main() {\ncallback fn (Str) Str=normalize\n}\n",
            "interface Worker {\n    fn start()\n    fn status() Str\n    fn callback() fn() Str\n}\nfn normalize(value Str) Str = value.trim()\nfn main() {\n    callback fn(Str) Str = normalize\n}\n",
        );
    }

    #[test]
    fn indentation_does_not_extend_an_unbraced_lambda() {
        let result = format(
            "fn main() Unit {\nmapper fn(Int) Int = value =>\n        value + 1\n        next = 2\nprintln(mapper(next))\n}\n",
        );
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
        assert_eq!(
            result.text,
            "fn main() {\n    mapper fn(Int) Int = value =>\n        value + 1\n    next = 2\n    println(mapper(next))\n}\n"
        );
    }

    #[test]
    fn preserves_multiline_string_contents() {
        let result = format(
            "fn main() Unit {\n    text = \"\"\"first\n  second   \nthird\"\"\"\nprintln(text)\n}",
        );
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
        assert_eq!(
            result.text,
            "fn main() {\n    text = \"\"\"first\n  second   \nthird\"\"\"\n    println(text)\n}\n"
        );
    }

    #[test]
    fn aligns_comma_separated_entries_and_closing_delimiters() {
        let result =
            format("fn main() Unit {\nvalues = [\n1,\n2\n]\npoint = {\nx: 1,\ny: 2\n}\n}\n");
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
        assert_eq!(
            result.text,
            "fn main() {\n    values = [\n        1,\n        2\n    ]\n    point = {\n        x: 1,\n        y: 2\n    }\n}\n"
        );
    }

    #[test]
    fn preserves_trailing_commas_in_multiline_lists() {
        let result = format(
            "fn combine(\nleft Int,\nright Int,\n) Int {\npair = (\nleft,\nright,\n)\npair[0] + pair[1]\n}\n",
        );
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
        assert_eq!(
            result.text,
            "fn combine(\n    left Int,\n    right Int,\n) Int {\n    pair = (\n        left,\n        right,\n    )\n    pair[0] + pair[1]\n}\n"
        );
    }

    #[test]
    fn normalizes_map_type_colon_spacing() {
        let result =
            format("fn main() Unit {\nspaced [Str : Int] = []\ncompact [Str:Int] = []\n}\n");
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
        assert_eq!(
            result.text,
            "fn main() {\n    spaced [Str: Int] = []\n    compact [Str: Int] = []\n}\n"
        );
    }

    #[test]
    fn formats_mixin_adoption_and_generic_type_arguments() {
        let source = "private mixin Parts[T] {\nfn keep(value Option[T]) Option[T] = value\n}\nclass Box {\nuse Parts[Option[Int]]\n}\n";
        let result = format(source);
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert!(result.text.contains("mixin Parts[T]"));
        assert!(!result.text.contains("private mixin"));
        assert!(result.text.contains("    use Parts[Int?]"));
        assert_eq!(format(&result.text).text, result.text);
    }

    #[test]
    fn formats_generic_transparent_aliases() {
        assert_formatted(
            "type Lookup[K with Hashed[K],V]=Map[K,Option[V]]\ntype Transform[A,B]=fn(A) Option[B]\n",
            "type Lookup[K with Hashed[K], V] = Map[K, V?]\ntype Transform[A, B] = fn(A) B?\n",
        );
    }

    #[test]
    fn preserves_interleaved_named_argument_and_shape_spread_order() {
        assert_formatted(
            "fn main() {\nreport(\"ready\",...options(),error=mark(5),...more())\nreport(\n\"ready\",\n...options(),\nerror=mark(5),\n...more(),\namount=mark(11)\n)\n}\n",
            "fn main() {\n    report(\"ready\", ...options(), error = mark(5), ...more())\n    report(\n        \"ready\",\n        ...options(),\n        error = mark(5),\n        ...more(),\n        amount = mark(11)\n    )\n}\n",
        );
    }

    #[test]
    fn formats_zero_argument_braced_lambdas() {
        let result = format(
            "fn invoke(work fn() Int) Int = work()\nfn main() {\nwork fn() Int={=>\nfirst=1\nfirst + 2\n}\ninvoke {=>\nwork()\n}\ninvoke({=>4})\n}\n",
        );
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert_eq!(
            result.text,
            "fn invoke(work fn() Int) Int = work()\nfn main() {\n    work fn() Int = { =>\n        first = 1\n        first + 2\n    }\n    invoke { =>\n        work()\n    }\n    invoke({ => 4 })\n}\n"
        );
        assert_eq!(format(&result.text).text, result.text);
    }

    #[test]
    fn formats_nested_trailing_lambdas_inside_calls() {
        let result = format(
            "class Aggregator {\nprivate fn get_rollup(key Str, range IntRange?) Rollup? = {\nfn notInRange(record Record) = {\nlet r <- range else return false\nrecord.timestamp < r.start || record.timestamp >= r.end\n}\nmaybeRollup = rollupMap[key].map { records =>\nrecords.fold(Rollup(0, 0), (acc, record) => {\nif record.void || notInRange(record) {\nreturn acc\n}\nRollup {\ntotal: record.amount + acc.total\ncount: acc.count + 1\n}\n})\n}\nif let rollup <- maybeRollup && rollup.count == 0 {\nreturn None\n}\nreturn maybeRollup\n}\n}\n",
        );
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
        assert_eq!(
            result.text,
            "class Aggregator {\n    private fn get_rollup(key Str, range IntRange?) Rollup? {\n        fn notInRange(record Record) {\n            r = range ?? return false\n            record.timestamp < r.start || record.timestamp >= r.end\n        }\n        maybeRollup = rollupMap[key].map { records =>\n            records.fold(Rollup(0, 0), (acc, record) => {\n                if record.void || notInRange(record) {\n                    return acc\n                }\n                Rollup {\n                    total: record.amount + acc.total\n                    count: acc.count + 1\n                }\n            })\n        }\n        if let rollup <- maybeRollup && rollup.count == 0 {\n            return None\n        }\n        return maybeRollup\n    }\n}\n"
        );
    }

    #[test]
    fn formats_braced_lambda_arguments_without_extra_call_indentation() {
        let result = format(
            "class Aggregator {\nfn all_rollups(range IntRange? = None) [Str] {\nrollups = this.rollups(range)\nrollups.sort((x, y) => {\nxRollup = x[1]\nyRollup = y[1]\nif xRollup.total == yRollup.total {\nreturn x[0].compareTo(y[0])\n}\nreturn yRollup.total.compareTo(xRollup.total)\n})\nrollups.map(x => \"${x[0]}(${x[1].count},${x[1].total})\")\n}\n}\n",
        );
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
        assert_eq!(
            result.text,
            "class Aggregator {\n    fn all_rollups(range IntRange? = None) [Str] {\n        rollups = this.rollups(range)\n        rollups.sort((x, y) => {\n            xRollup = x[1]\n            yRollup = y[1]\n            if xRollup.total == yRollup.total {\n                return x[0].compareTo(y[0])\n            }\n            return yRollup.total.compareTo(xRollup.total)\n        })\n        rollups.map(x => \"${x[0]}(${x[1].count},${x[1].total})\")\n    }\n}\n"
        );
    }

    #[test]
    fn carries_expression_continuation_through_a_trailing_lambda() {
        let result = format(
            "fn readString(path Str) Result[Str, Str] =\nFile.readText(path).mapError { err =>\nfileErrorMessage(err)\n}\n",
        );
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
        assert_eq!(
            result.text,
            "fn readString(path Str) Result[Str, Str] =\n    File.readText(path).mapError { err =>\n        fileErrorMessage(err)\n    }\n"
        );
    }

    #[test]
    fn formats_import_wildcards_multiline_calls_and_grouped_for() {
        let result = format(
            "use Models/*\nuse runtime/EventRuntime\n\nprivate fn category(value Str?) Bool =\nEventRuntime.containsRegex(\nvalue ?? \"\",\nraw\"(?i)crypto\"\n)\n\nfn pairs(left [Int], right [Int]) Unit {\nfor {\nitem <- left\nother <- right\n} do {\nprintln(item, other)\n}\n}\n",
        );
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
        assert_eq!(
            result.text,
            "use Models/*\nuse runtime/EventRuntime\n\nfn category(value Str?) Bool =\n    EventRuntime.containsRegex(\n        value ?? \"\",\n        raw\"(?i)crypto\"\n    )\n\nfn pairs(left [Int], right [Int]) {\n    for {\n        item <- left\n        other <- right\n    } do {\n        println(item, other)\n    }\n}\n"
        );
    }

    #[test]
    fn removes_only_visibility_modifiers_that_match_the_scope_default() {
        let result = format(
            "private fn helper() Unit {}\npublic fn api() Unit {}\nprivate type Name = Str\nprivate class Hidden {\npublic value Int\nprivate secret Int\npublic get visible Int = value\npublic fn read() Int = value\n}\n",
        );
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
        assert_eq!(
            result.text,
            "fn helper() {}\npublic fn api() {}\ntype Name = Str\nclass Hidden {\n    value Int\n    private secret Int\n    get visible Int = value\n    fn read() Int = value\n}\n"
        );
    }

    #[test]
    fn preserves_whole_module_aliases() {
        assert_formatted(
            "use customers / storage  as  customerStore\nuse audit/storage as auditStore\nuse models/User as Customer\nfn main() {\ncustomerStore.load(1)\n}\n",
            "use customers/storage as customerStore\nuse audit/storage as auditStore\nuse models/User as Customer\nfn main() {\n    customerStore.load(1)\n}\n",
        );
    }

    #[test]
    fn preserves_selective_import_syntax() {
        let result = format(
            "use providers / common / { canonicalResolution, canonicalStatus, SourceIngestor }\nuse lume/json/{Json, JsonValue}\n",
        );
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
        assert_eq!(
            result.text,
            "use providers/common/{canonicalResolution, canonicalStatus, SourceIngestor}\nuse lume/json/{Json, JsonValue}\n"
        );

        let reparsed_file = SourceFile::new("formatted.lum", result.text.clone());
        let reparsed_tokens = lex(&reparsed_file);
        assert!(
            reparsed_tokens.diagnostics.is_empty(),
            "{:#?}",
            reparsed_tokens.diagnostics
        );
        let reparsed = parse_program(&reparsed_tokens.tokens);
        assert!(
            reparsed.diagnostics.is_empty(),
            "{:#?}",
            reparsed.diagnostics
        );

        let reformatted = format(&result.text);
        assert_eq!(reformatted.text, result.text);
    }

    #[test]
    fn formats_single_statement_grouped_for_body_without_extending_its_scope() {
        let result = format(
            "fn pairs(left [Int], right [Int]) Unit {\nfor {\nsource <- left\ntarget <- right\n} do\nif source != target {\nprintln(source, target)\n}\nprintln(\"done\")\n}\n",
        );
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
        assert_eq!(
            result.text,
            "fn pairs(left [Int], right [Int]) {\n    for {\n        source <- left\n        target <- right\n    } do\n        if source != target {\n            println(source, target)\n        }\n    println(\"done\")\n}\n"
        );
    }

    #[test]
    fn formats_single_generator_do_bodies() {
        let result = format(
            "fn visit(values [Int]) Unit {\nfor value <- values do\nprintln(value)\nfor value <- values do {\nprintln(value)\n}\n}\n",
        );
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
        assert_eq!(
            result.text,
            "fn visit(values [Int]) {\n    for value <- values do\n        println(value)\n    for value <- values do {\n        println(value)\n    }\n}\n"
        );
    }

    #[test]
    fn inline_record_argument_does_not_consume_lambda_indentation() {
        let result = format(
            "fn main() Unit {\nargs { verbose Bool, help Bool }=\nOS.args.fold({verbose: false, help : false}, (acc, arg) => {\nif arg == \"--verbose\" {\n{...acc, verbose: true}\n} else if arg == \"--help\" {\n{...acc, help: true}\n} else {\nacc\n}\n})\n}\n",
        );
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
        assert_eq!(
            result.text,
            "fn main() {\n    args { verbose Bool, help Bool } =\n        OS.args.fold({ verbose: false, help: false }, (acc, arg) => {\n            if arg == \"--verbose\" {\n                { ...acc, verbose: true }\n            } else if arg == \"--help\" {\n                { ...acc, help: true }\n            } else {\n                acc\n            }\n        })\n}\n"
        );
    }

    #[test]
    fn normalizes_horizontal_spacing_without_changing_literals_or_comments() {
        let result = format(
            "class Aggregator {\nprivate rollupMap [Str :  [Record]] = []\nprivate  recordMap [Str: Record] = []\nget coordinates  (Int, Int) = ( 1, 2 )\nfn label() Str {\nmessage  =  \"keep  spaces\" # keep  comment spacing\nprintln( \"kind\", message )\nmessage\n}\n}\n",
        );
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
        assert_eq!(
            result.text,
            "class Aggregator {\n    private rollupMap [Str: [Record]] = []\n    private recordMap [Str: Record] = []\n    get coordinates (Int, Int) = (1, 2)\n    fn label() Str {\n        message = \"keep  spaces\" # keep  comment spacing\n        println(\"kind\", message)\n        message\n    }\n}\n"
        );
    }

    #[test]
    fn preserves_equals_before_assignment_style_callable_body() {
        let result = format(
            "class Counter {\nprivate var value Int = 0\nfn reset() Unit =\nthis.value := 0\n}\n",
        );
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert_eq!(
            result.text,
            "class Counter {\n    private var value Int = 0\n    fn reset() =\n        this.value := 0\n}\n"
        );
        assert_eq!(format(&result.text).text, result.text);
        assert!(format(&result.text).diagnostics.is_empty());
    }

    #[test]
    fn canonicalizes_unit_returns_and_callable_block_bodies() {
        let result = format(
            "interface Action {\nfn run() Unit\nget invalid Unit\n}\nclass Worker {\nfn run() Unit = {\nprintln(\"run\")\n}\nfn answer() Int = {\n42\n}\nget count Int = {\n1\n}\nnew() = {\nprintln(\"new\")\n}\n}\nfn callback() fn() Unit = () => ()\nfn make() { x Int } = { x: 1 }\n",
        );
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
        assert_eq!(
            result.text,
            "interface Action {\n    fn run()\n    get invalid Unit\n}\nclass Worker {\n    fn run() {\n        println(\"run\")\n    }\n    fn answer() Int {\n        42\n    }\n    get count Int {\n        1\n    }\n    new() {\n        println(\"new\")\n    }\n}\nfn callback() fn() Unit = () => ()\nfn make() { x Int } = { x: 1 }\n"
        );
    }

    #[test]
    fn invalid_source_is_not_rewritten() {
        let source = "fn main( Unit {\n";
        let result = format(source);
        assert!(result.has_errors());
        assert_eq!(result.text, source);
    }

    #[test]
    fn formatting_is_idempotent() {
        let first = format("fn main() Unit {\nprintln(\"ok\")\n}");
        assert!(!first.has_errors());
        let second = format(&first.text);
        assert!(!second.has_errors());
        assert_eq!(second.text, first.text);
    }
}
