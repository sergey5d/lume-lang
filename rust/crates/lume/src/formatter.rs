use crate::{
    Diagnostic, Keyword, SourceFile, Token, TokenKind,
    ast::{
        Annotation, CallableBody, DeferAction, ElseBranch, ElseExprBranch, Expr, ForBinding,
        FunctionDecl, IfConditionClause, Item, LambdaBody, MatchCase, MatchCaseBody, MethodDecl,
        Program, Stmt, TypeMember, TypeRef,
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
/// while applying the language's canonical callable spelling. Invalid input is
/// returned unchanged so callers can safely avoid overwriting the source file.
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
    let canonical_source = canonicalize_callable_syntax(&file.text, &lexed.tokens, &program);
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

fn canonicalize_callable_syntax(source: &str, tokens: &[Token], program: &Program) -> String {
    let mut removals = Vec::new();
    collect_program_rewrites(program, tokens, &mut removals);
    removals.sort_by_key(|span| (span.start, span.end));
    removals.dedup_by_key(|span| (span.start, span.end));

    let mut result = source.to_string();
    for span in removals.into_iter().rev() {
        result.replace_range(span.start..span.end, "");
    }
    result
}

fn collect_program_rewrites(program: &Program, tokens: &[Token], removals: &mut Vec<Span>) {
    for item in &program.items {
        match item {
            Item::Function(function) => collect_function_rewrites(function, tokens, removals),
            Item::Type(decl) => {
                for annotation in &decl.annotations {
                    collect_annotation_rewrites(annotation, tokens, removals);
                }
                for member in &decl.members {
                    match member {
                        TypeMember::Field(field) => {
                            for annotation in &field.annotations {
                                collect_annotation_rewrites(annotation, tokens, removals);
                            }
                            if let Some(initializer) = &field.initializer {
                                collect_expr_rewrites(initializer, tokens, removals);
                            }
                        }
                        TypeMember::Method(method) => {
                            collect_method_rewrites(method, tokens, removals)
                        }
                        TypeMember::Case(case) => {
                            for field in &case.fields {
                                for annotation in &field.annotations {
                                    collect_annotation_rewrites(annotation, tokens, removals);
                                }
                                if let Some(initializer) = &field.initializer {
                                    collect_expr_rewrites(initializer, tokens, removals);
                                }
                            }
                        }
                    }
                }
            }
            Item::Extension(extension) => {
                for method in &extension.methods {
                    collect_method_rewrites(method, tokens, removals);
                }
            }
            Item::Statement(stmt) => collect_stmt_rewrites(stmt, tokens, removals),
            Item::TypeAlias(_) => {}
        }
    }
}

fn collect_annotation_rewrites(
    annotation: &Annotation,
    tokens: &[Token],
    removals: &mut Vec<Span>,
) {
    collect_expr_rewrites(&annotation.value, tokens, removals);
}

fn collect_function_rewrites(function: &FunctionDecl, tokens: &[Token], removals: &mut Vec<Span>) {
    collect_explicit_unit_return(&function.return_type, removals);
    for annotation in &function.annotations {
        collect_annotation_rewrites(annotation, tokens, removals);
    }
    for param in &function.params {
        if let Some(initializer) = &param.initializer {
            collect_expr_rewrites(initializer, tokens, removals);
        }
    }
    collect_callable_body_rewrites(&function.body, function.span, tokens, removals);
}

fn collect_method_rewrites(method: &MethodDecl, tokens: &[Token], removals: &mut Vec<Span>) {
    if !method.getter {
        collect_explicit_unit_return(&method.return_type, removals);
    }
    for annotation in &method.annotations {
        collect_annotation_rewrites(annotation, tokens, removals);
    }
    for param in &method.params {
        if let Some(initializer) = &param.initializer {
            collect_expr_rewrites(initializer, tokens, removals);
        }
    }
    if let Some(body) = &method.body {
        collect_callable_body_rewrites(body, method.span, tokens, removals);
    }
}

fn collect_explicit_unit_return(return_type: &Option<TypeRef>, removals: &mut Vec<Span>) {
    if let Some(TypeRef::Named { name, args, span }) = return_type
        && name == "Unit"
        && args.is_empty()
    {
        removals.push(*span);
    }
}

fn collect_callable_body_rewrites(
    body: &CallableBody,
    callable_span: Span,
    tokens: &[Token],
    removals: &mut Vec<Span>,
) {
    match body {
        CallableBody::Block(block) => {
            if let Some(equals) = token_immediately_before(tokens, block.span.start)
                && equals.kind == TokenKind::Eq
                && equals.span.start >= callable_span.start
            {
                removals.push(equals.span);
            }
            collect_block_rewrites(block, tokens, removals);
        }
        CallableBody::Expr(expr) => collect_expr_rewrites(expr, tokens, removals),
    }
}

fn token_immediately_before(tokens: &[Token], offset: usize) -> Option<&Token> {
    tokens
        .iter()
        .take_while(|token| token.span.end <= offset)
        .filter(|token| !matches!(token.kind, TokenKind::Newline | TokenKind::Eof))
        .last()
}

fn collect_block_rewrites(block: &crate::ast::Block, tokens: &[Token], removals: &mut Vec<Span>) {
    for statement in &block.statements {
        collect_stmt_rewrites(statement, tokens, removals);
    }
}

fn collect_clause_rewrites(
    clause: &crate::ast::RefutableClause,
    tokens: &[Token],
    removals: &mut Vec<Span>,
) {
    collect_expr_rewrites(&clause.value, tokens, removals);
}

fn collect_condition_clause_rewrites(
    clause: &IfConditionClause,
    tokens: &[Token],
    removals: &mut Vec<Span>,
) {
    match clause {
        IfConditionClause::Let(clause) => collect_clause_rewrites(clause, tokens, removals),
        IfConditionClause::Expr(expr) => collect_expr_rewrites(expr, tokens, removals),
    }
}

fn collect_match_case_rewrites(case: &MatchCase, tokens: &[Token], removals: &mut Vec<Span>) {
    if let Some(guard) = &case.guard {
        collect_expr_rewrites(guard, tokens, removals);
    }
    match &case.body {
        MatchCaseBody::Block(block) => collect_block_rewrites(block, tokens, removals),
        MatchCaseBody::Expr(expr) => collect_expr_rewrites(expr, tokens, removals),
    }
}

fn collect_for_binding_rewrites(binding: &ForBinding, tokens: &[Token], removals: &mut Vec<Span>) {
    if let Some(iterable) = &binding.iterable {
        collect_expr_rewrites(iterable, tokens, removals);
    }
    for value in &binding.values {
        collect_expr_rewrites(value, tokens, removals);
    }
}

fn collect_stmt_rewrites(stmt: &Stmt, tokens: &[Token], removals: &mut Vec<Span>) {
    match stmt {
        Stmt::Binding(stmt) => {
            for value in &stmt.values {
                collect_expr_rewrites(value, tokens, removals);
            }
        }
        Stmt::PatternBinding(stmt) => {
            for clause in &stmt.clauses {
                collect_clause_rewrites(clause, tokens, removals);
            }
            collect_expr_rewrites(&stmt.value, tokens, removals);
        }
        Stmt::Assignment(stmt) => {
            for target in &stmt.targets {
                collect_expr_rewrites(target, tokens, removals);
            }
            for value in &stmt.values {
                collect_expr_rewrites(value, tokens, removals);
            }
        }
        Stmt::Defer(stmt) => match &stmt.action {
            DeferAction::Call(expr) => collect_expr_rewrites(expr, tokens, removals),
            DeferAction::Block(block) => collect_block_rewrites(block, tokens, removals),
        },
        Stmt::If(stmt) => {
            if let Some(condition) = &stmt.condition {
                collect_expr_rewrites(condition, tokens, removals);
            }
            for clause in &stmt.condition_clauses {
                collect_condition_clause_rewrites(clause, tokens, removals);
            }
            if let Some(value) = &stmt.pattern_value {
                collect_expr_rewrites(value, tokens, removals);
            }
            for clause in &stmt.pattern_clauses {
                collect_clause_rewrites(clause, tokens, removals);
            }
            if let Some(value) = &stmt.binding_value {
                collect_expr_rewrites(value, tokens, removals);
            }
            collect_block_rewrites(&stmt.then_block, tokens, removals);
            if let Some(branch) = &stmt.else_branch {
                match branch {
                    ElseBranch::If(stmt) => {
                        collect_stmt_rewrites(&Stmt::If((**stmt).clone()), tokens, removals)
                    }
                    ElseBranch::Block(block) => collect_block_rewrites(block, tokens, removals),
                }
            }
        }
        Stmt::Match(stmt) => {
            collect_expr_rewrites(&stmt.value, tokens, removals);
            for case in &stmt.cases {
                collect_match_case_rewrites(case, tokens, removals);
            }
        }
        Stmt::While(stmt) => {
            for clause in &stmt.condition_clauses {
                collect_condition_clause_rewrites(clause, tokens, removals);
            }
            collect_block_rewrites(&stmt.body, tokens, removals);
        }
        Stmt::For(stmt) => {
            for binding in &stmt.bindings {
                collect_for_binding_rewrites(binding, tokens, removals);
            }
            collect_block_rewrites(&stmt.body, tokens, removals);
        }
        Stmt::LetElse(stmt) => {
            for clause in &stmt.clauses {
                collect_clause_rewrites(clause, tokens, removals);
            }
            collect_expr_rewrites(&stmt.value, tokens, removals);
            collect_block_rewrites(&stmt.else_block, tokens, removals);
        }
        Stmt::Return(stmt) => {
            if let Some(value) = &stmt.value {
                collect_expr_rewrites(value, tokens, removals);
            }
        }
        Stmt::Expr(stmt) => collect_expr_rewrites(&stmt.expr, tokens, removals),
        Stmt::LocalFunction(function) => collect_function_rewrites(function, tokens, removals),
        Stmt::Break(_) | Stmt::Continue(_) => {}
    }
}

fn collect_expr_rewrites(expr: &Expr, tokens: &[Token], removals: &mut Vec<Span>) {
    match expr {
        Expr::Identifier { .. }
        | Expr::Placeholder { .. }
        | Expr::Integer { .. }
        | Expr::Float { .. }
        | Expr::String { .. }
        | Expr::Bool { .. }
        | Expr::Unit { .. }
        | Expr::Break { .. }
        | Expr::Continue { .. }
        | Expr::TypeOf { .. } => {}
        Expr::Spread { value, .. } | Expr::Try { value, .. } => {
            collect_expr_rewrites(value, tokens, removals)
        }
        Expr::ListLiteral { items, .. } | Expr::TupleLiteral { items, .. } => {
            for item in items {
                collect_expr_rewrites(item, tokens, removals);
            }
        }
        Expr::Call { callee, args, .. } => {
            collect_expr_rewrites(callee, tokens, removals);
            for arg in args {
                collect_expr_rewrites(&arg.value, tokens, removals);
            }
        }
        Expr::ContextualNew { args, .. } => {
            for arg in args {
                collect_expr_rewrites(&arg.value, tokens, removals);
            }
        }
        Expr::Member { receiver, .. } => collect_expr_rewrites(receiver, tokens, removals),
        Expr::Index {
            receiver, index, ..
        } => {
            collect_expr_rewrites(receiver, tokens, removals);
            collect_expr_rewrites(index, tokens, removals);
        }
        Expr::RecordLiteral { fields, values, .. } => {
            for field in fields {
                collect_expr_rewrites(&field.value, tokens, removals);
            }
            for value in values {
                collect_expr_rewrites(value, tokens, removals);
            }
        }
        Expr::AnonymousObject {
            fields, methods, ..
        } => {
            for field in fields {
                for annotation in &field.annotations {
                    collect_annotation_rewrites(annotation, tokens, removals);
                }
                if let Some(initializer) = &field.initializer {
                    collect_expr_rewrites(initializer, tokens, removals);
                }
            }
            for method in methods {
                collect_method_rewrites(method, tokens, removals);
            }
        }
        Expr::ExtractOr {
            value, fallback, ..
        } => {
            collect_expr_rewrites(value, tokens, removals);
            collect_expr_rewrites(fallback, tokens, removals);
        }
        Expr::Return { value, .. } => {
            if let Some(value) = value {
                collect_expr_rewrites(value, tokens, removals);
            }
        }
        Expr::Unary { expr, .. } | Expr::Group { inner: expr, .. } => {
            collect_expr_rewrites(expr, tokens, removals)
        }
        Expr::Binary { left, right, .. } => {
            collect_expr_rewrites(left, tokens, removals);
            collect_expr_rewrites(right, tokens, removals);
        }
        Expr::Is { left, .. } => collect_expr_rewrites(left, tokens, removals),
        Expr::If {
            condition_clauses,
            then_block,
            else_branch,
            ..
        } => {
            for clause in condition_clauses {
                collect_condition_clause_rewrites(clause, tokens, removals);
            }
            collect_block_rewrites(then_block, tokens, removals);
            match else_branch.as_ref() {
                ElseExprBranch::If(expr) => collect_expr_rewrites(expr, tokens, removals),
                ElseExprBranch::Block(block) => collect_block_rewrites(block, tokens, removals),
            }
        }
        Expr::Block { body, .. } => collect_block_rewrites(body, tokens, removals),
        Expr::Match { value, cases, .. } => {
            collect_expr_rewrites(value, tokens, removals);
            for case in cases {
                collect_match_case_rewrites(case, tokens, removals);
            }
        }
        Expr::ForYield {
            bindings,
            yield_body,
            ..
        } => {
            for binding in bindings {
                collect_for_binding_rewrites(binding, tokens, removals);
            }
            collect_block_rewrites(yield_body, tokens, removals);
        }
        Expr::Lambda { body, .. } => match body {
            LambdaBody::Expr(expr) => collect_expr_rewrites(expr, tokens, removals),
            LambdaBody::Block(block) => collect_block_rewrites(block, tokens, removals),
        },
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
        continuation_pending = line_requests_continuation(line_tokens);
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

    if current.kind == TokenKind::LParen && can_end_callee(previous.kind) {
        return is_parenthesized_getter_return(tokens, current_index);
    }

    if current.kind == TokenKind::LBracket {
        return had_space;
    }

    had_space || requires_horizontal_space(previous.kind, current.kind)
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

    #[test]
    fn formats_blocks_continuations_and_trailing_whitespace() {
        let result = format(
            "def main() Unit {\nprintln(\"start\")  \nvalue = 1 +\n2\nif true {\nprintln(value)\n}\n}\n",
        );
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
        assert_eq!(
            result.text,
            "def main() {\n    println(\"start\")\n    value = 1 +\n        2\n    if true {\n        println(value)\n    }\n}\n"
        );
    }

    #[test]
    fn canonicalizes_spaces_before_callable_parameter_lists() {
        let result = format(
            "def calculate (input Int) Int = input\ndef identity[T] (input T) T = input\nclass Calculator {\ndef calculate (input Int) Int = input\ndef convert[T] () T = panic(\"missing\")\n}\n",
        );
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
        assert_eq!(
            result.text,
            "def calculate(input Int) Int = input\ndef identity[T](input T) T = input\nclass Calculator {\n    def calculate(input Int) Int = input\n    def convert[T]() T = panic(\"missing\")\n}\n"
        );
    }

    #[test]
    fn indentation_does_not_extend_an_unbraced_lambda() {
        let result = format(
            "def main() Unit {\nmapper fn(Int) Int = value =>\n        value + 1\n        next = 2\nprintln(mapper(next))\n}\n",
        );
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
        assert_eq!(
            result.text,
            "def main() {\n    mapper fn(Int) Int = value =>\n        value + 1\n    next = 2\n    println(mapper(next))\n}\n"
        );
    }

    #[test]
    fn preserves_multiline_string_contents() {
        let result = format(
            "def main() Unit {\n    text = \"\"\"first\n  second   \nthird\"\"\"\nprintln(text)\n}",
        );
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
        assert_eq!(
            result.text,
            "def main() {\n    text = \"\"\"first\n  second   \nthird\"\"\"\n    println(text)\n}\n"
        );
    }

    #[test]
    fn aligns_comma_separated_entries_and_closing_delimiters() {
        let result =
            format("def main() Unit {\nvalues = [\n1,\n2\n]\npoint = {\nx: 1,\ny: 2\n}\n}\n");
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
        assert_eq!(
            result.text,
            "def main() {\n    values = [\n        1,\n        2\n    ]\n    point = {\n        x: 1,\n        y: 2\n    }\n}\n"
        );
    }

    #[test]
    fn preserves_trailing_commas_in_multiline_lists() {
        let result = format(
            "def combine(\nleft Int,\nright Int,\n) Int {\npair = (\nleft,\nright,\n)\npair[0] + pair[1]\n}\n",
        );
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
        assert_eq!(
            result.text,
            "def combine(\n    left Int,\n    right Int,\n) Int {\n    pair = (\n        left,\n        right,\n    )\n    pair[0] + pair[1]\n}\n"
        );
    }

    #[test]
    fn normalizes_map_type_colon_spacing() {
        let result =
            format("def main() Unit {\nspaced [Str : Int] = []\ncompact [Str:Int] = []\n}\n");
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
        assert_eq!(
            result.text,
            "def main() {\n    spaced [Str: Int] = []\n    compact [Str: Int] = []\n}\n"
        );
    }

    #[test]
    fn formats_nested_trailing_lambdas_inside_calls() {
        let result = format(
            "class Aggregator {\nprivate def get_rollup(key Str, range IntRange?) Rollup? = {\ndef notInRange(record Record) = {\nlet r <- range else return false\nrecord.timestamp < r.start || record.timestamp >= r.end\n}\nmaybeRollup = rollupMap[key].map { records =>\nrecords.fold(Rollup(0, 0), (acc, record) => {\nif record.void || notInRange(record) {\nreturn acc\n}\nRollup {\ntotal: record.amount + acc.total\ncount: acc.count + 1\n}\n})\n}\nif let rollup <- maybeRollup && rollup.count == 0 {\nreturn None\n}\nreturn maybeRollup\n}\n}\n",
        );
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
        assert_eq!(
            result.text,
            "class Aggregator {\n    private def get_rollup(key Str, range IntRange?) Rollup? {\n        def notInRange(record Record) {\n            let r <- range else return false\n            record.timestamp < r.start || record.timestamp >= r.end\n        }\n        maybeRollup = rollupMap[key].map { records =>\n            records.fold(Rollup(0, 0), (acc, record) => {\n                if record.void || notInRange(record) {\n                    return acc\n                }\n                Rollup {\n                    total: record.amount + acc.total\n                    count: acc.count + 1\n                }\n            })\n        }\n        if let rollup <- maybeRollup && rollup.count == 0 {\n            return None\n        }\n        return maybeRollup\n    }\n}\n"
        );
    }

    #[test]
    fn formats_braced_lambda_arguments_without_extra_call_indentation() {
        let result = format(
            "class Aggregator {\ndef all_rollups(range IntRange? = None) [Str] {\nrollups = this.rollups(range)\nrollups.sort((x, y) => {\nxRollup = x[1]\nyRollup = y[1]\nif xRollup.total == yRollup.total {\nreturn x[0].compareTo(y[0])\n}\nreturn yRollup.total.compareTo(xRollup.total)\n})\nrollups.map(x => \"${x[0]}(${x[1].count},${x[1].total})\")\n}\n}\n",
        );
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
        assert_eq!(
            result.text,
            "class Aggregator {\n    def all_rollups(range IntRange? = None) [Str] {\n        rollups = this.rollups(range)\n        rollups.sort((x, y) => {\n            xRollup = x[1]\n            yRollup = y[1]\n            if xRollup.total == yRollup.total {\n                return x[0].compareTo(y[0])\n            }\n            return yRollup.total.compareTo(xRollup.total)\n        })\n        rollups.map(x => \"${x[0]}(${x[1].count},${x[1].total})\")\n    }\n}\n"
        );
    }

    #[test]
    fn carries_expression_continuation_through_a_trailing_lambda() {
        let result = format(
            "def readString(path Str) Result[Str, Str] =\nFile.readText(path).mapError { err =>\nfileErrorMessage(err)\n}\n",
        );
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
        assert_eq!(
            result.text,
            "def readString(path Str) Result[Str, Str] =\n    File.readText(path).mapError { err =>\n        fileErrorMessage(err)\n    }\n"
        );
    }

    #[test]
    fn formats_import_wildcards_multiline_calls_and_grouped_for() {
        let result = format(
            "use Models/*\nuse runtime/EventRuntime\n\nprivate def category(value Str?) Bool =\nEventRuntime.containsRegex(\nvalue ?? \"\",\nraw\"(?i)crypto\"\n)\n\ndef pairs(left [Int], right [Int]) Unit {\nfor {\nitem <- left\nother <- right\n} do {\nprintln(item, other)\n}\n}\n",
        );
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
        assert_eq!(
            result.text,
            "use Models/*\nuse runtime/EventRuntime\n\nprivate def category(value Str?) Bool =\n    EventRuntime.containsRegex(\n        value ?? \"\",\n        raw\"(?i)crypto\"\n    )\n\ndef pairs(left [Int], right [Int]) {\n    for {\n        item <- left\n        other <- right\n    } do {\n        println(item, other)\n    }\n}\n"
        );
    }

    #[test]
    fn formats_single_statement_grouped_for_body_without_extending_its_scope() {
        let result = format(
            "def pairs(left [Int], right [Int]) Unit {\nfor {\nsource <- left\ntarget <- right\n} do\nif source != target {\nprintln(source, target)\n}\nprintln(\"done\")\n}\n",
        );
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
        assert_eq!(
            result.text,
            "def pairs(left [Int], right [Int]) {\n    for {\n        source <- left\n        target <- right\n    } do\n        if source != target {\n            println(source, target)\n        }\n    println(\"done\")\n}\n"
        );
    }

    #[test]
    fn formats_single_generator_do_bodies() {
        let result = format(
            "def visit(values [Int]) Unit {\nfor value <- values do\nprintln(value)\nfor value <- values do {\nprintln(value)\n}\n}\n",
        );
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
        assert_eq!(
            result.text,
            "def visit(values [Int]) {\n    for value <- values do\n        println(value)\n    for value <- values do {\n        println(value)\n    }\n}\n"
        );
    }

    #[test]
    fn inline_record_argument_does_not_consume_lambda_indentation() {
        let result = format(
            "def main() Unit {\nargs { verbose Bool, help Bool }=\nOS.args.fold({verbose: false, help : false}, (acc, arg) => {\nif arg == \"--verbose\" {\n{...acc, verbose: true}\n} else if arg == \"--help\" {\n{...acc, help: true}\n} else {\nacc\n}\n})\n}\n",
        );
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
        assert_eq!(
            result.text,
            "def main() {\n    args { verbose Bool, help Bool } =\n        OS.args.fold({ verbose: false, help: false }, (acc, arg) => {\n            if arg == \"--verbose\" {\n                { ...acc, verbose: true }\n            } else if arg == \"--help\" {\n                { ...acc, help: true }\n            } else {\n                acc\n            }\n        })\n}\n"
        );
    }

    #[test]
    fn normalizes_horizontal_spacing_without_changing_literals_or_comments() {
        let result = format(
            "class Aggregator {\nprivate rollupMap [Str :  [Record]] = []\nprivate  recordMap [Str: Record] = []\nget coordinates  (Int, Int) = ( 1, 2 )\ndef label() Str {\nmessage  =  \"keep  spaces\" # keep  comment spacing\nprintln( \"kind\", message )\nmessage\n}\n}\n",
        );
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
        assert_eq!(
            result.text,
            "class Aggregator {\n    private rollupMap [Str: [Record]] = []\n    private recordMap [Str: Record] = []\n    get coordinates (Int, Int) = (1, 2)\n    def label() Str {\n        message = \"keep  spaces\" # keep  comment spacing\n        println(\"kind\", message)\n        message\n    }\n}\n"
        );
    }

    #[test]
    fn canonicalizes_unit_returns_and_callable_block_bodies() {
        let result = format(
            "interface Action {\ndef run() Unit\nget invalid Unit\n}\nclass Worker {\ndef run() Unit = {\nprintln(\"run\")\n}\ndef answer() Int = {\n42\n}\nget count Int = {\n1\n}\nnew() = {\nprintln(\"new\")\n}\n}\ndef callback() fn() Unit = () => ()\ndef make() { x Int } = { x: 1 }\n",
        );
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
        assert_eq!(
            result.text,
            "interface Action {\n    def run()\n    get invalid Unit\n}\nclass Worker {\n    def run() {\n        println(\"run\")\n    }\n    def answer() Int {\n        42\n    }\n    get count Int {\n        1\n    }\n    new() {\n        println(\"new\")\n    }\n}\ndef callback() fn() Unit = () => ()\ndef make() { x Int } = { x: 1 }\n"
        );
    }

    #[test]
    fn invalid_source_is_not_rewritten() {
        let source = "def main( Unit {\n";
        let result = format(source);
        assert!(result.has_errors());
        assert_eq!(result.text, source);
    }

    #[test]
    fn formatting_is_idempotent() {
        let first = format("def main() Unit {\nprintln(\"ok\")\n}");
        assert!(!first.has_errors());
        let second = format(&first.text);
        assert!(!second.has_errors());
        assert_eq!(second.text, first.text);
    }
}
