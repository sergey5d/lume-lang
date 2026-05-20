use crate::{
    ast::{
        AssignOp, AssignmentStmt, BinaryOp, Binding, BindingStmt, Block, BreakStmt, CallArg,
        CallableBody, ElseBranch, ElseExprBranch, EnumCaseDecl, Expr, ExprStmt, FieldDecl, ForStmt,
        FunctionDecl, IfStmt, ImplBlock, ImportDecl, Item, LambdaBody, LambdaParam, MethodDecl,
        PackageDecl, Param, Program, ReturnStmt, Stmt, TupleTypeField, TypeDecl, TypeKind,
        TypeMember, TypeParam, TypeRef, UnaryOp, Visibility, WhileStmt,
    },
    diagnostic::Diagnostic,
    lexer::{Keyword, Token, TokenKind},
    source::Span,
};

pub struct ParseResult {
    pub program: Option<Program>,
    pub diagnostics: Vec<Diagnostic>,
}

pub fn parse_program(tokens: &[Token]) -> ParseResult {
    Parser::new(tokens).parse_program()
}

struct Parser<'a> {
    tokens: &'a [Token],
    index: usize,
    diagnostics: Vec<Diagnostic>,
}

#[derive(Clone, Copy)]
struct Checkpoint {
    index: usize,
    diagnostics_len: usize,
}

impl<'a> Parser<'a> {
    fn new(tokens: &'a [Token]) -> Self {
        Self {
            tokens,
            index: 0,
            diagnostics: Vec::new(),
        }
    }

    fn parse_program(mut self) -> ParseResult {
        self.skip_newlines();
        let start_span = self.current_span();

        let package = if self.match_keyword(Keyword::Package) {
            self.parse_package_decl()
        } else {
            None
        };

        self.skip_newlines();
        let mut imports = Vec::new();
        while self.match_keyword(Keyword::Import) {
            if let Some(import) = self.parse_import_decl() {
                imports.push(import);
            }
            self.skip_newlines();
        }

        let mut items = Vec::new();
        while !self.at(TokenKind::Eof) {
            self.skip_newlines();
            if self.at(TokenKind::Eof) {
                break;
            }

            let before = self.index;
            if let Some(item) = self.parse_item() {
                items.push(item);
            } else {
                self.synchronize_item();
                if self.index == before && !self.at(TokenKind::Eof) {
                    self.advance();
                }
            }
            self.skip_newlines();
        }

        let end_span = if self.index > 0 {
            self.tokens[self.index.saturating_sub(1)].span
        } else {
            start_span
        };

        let program = Program {
            package,
            imports,
            items,
            span: Some(start_span.cover(end_span)),
        };

        ParseResult {
            program: Some(program),
            diagnostics: self.diagnostics,
        }
    }

    fn parse_package_decl(&mut self) -> Option<PackageDecl> {
        let start = self.previous_span();
        let name = self.parse_path_string()?;
        let end = self.last_non_newline_span(start);
        Some(PackageDecl {
            name,
            span: start.cover(end),
        })
    }

    fn parse_import_decl(&mut self) -> Option<ImportDecl> {
        let start = self.previous_span();
        let path = self.parse_path_string()?;
        let end = self.last_non_newline_span(start);
        Some(ImportDecl {
            path,
            span: start.cover(end),
        })
    }

    fn parse_item(&mut self) -> Option<Item> {
        let visibility = self.parse_visibility();
        match self.current_kind() {
            TokenKind::Keyword(Keyword::Def) => {
                let function = self.parse_function_decl(visibility)?;
                Some(Item::Function(function))
            }
            TokenKind::Keyword(Keyword::Class)
            | TokenKind::Keyword(Keyword::Record)
            | TokenKind::Keyword(Keyword::Object)
            | TokenKind::Keyword(Keyword::Interface)
            | TokenKind::Keyword(Keyword::Enum) => {
                let decl = self.parse_type_decl(visibility)?;
                Some(Item::Type(decl))
            }
            TokenKind::Keyword(Keyword::Impl) => {
                if visibility != Visibility::Default {
                    self.error_at_current(
                        "unexpected_visibility",
                        "impl blocks do not accept visibility modifiers",
                    );
                    return None;
                }
                let block = self.parse_impl_block()?;
                Some(Item::Impl(block))
            }
            _ => {
                if visibility != Visibility::Default {
                    self.error_at_current(
                        "unexpected_visibility",
                        "visibility modifiers are only valid on declarations",
                    );
                    return None;
                }
                self.parse_stmt().map(Item::Statement)
            }
        }
    }

    fn parse_visibility(&mut self) -> Visibility {
        if self.match_keyword(Keyword::Public) {
            Visibility::Public
        } else if self.match_keyword(Keyword::Hidden) {
            Visibility::Hidden
        } else {
            Visibility::Default
        }
    }

    fn parse_function_decl(&mut self, visibility: Visibility) -> Option<FunctionDecl> {
        let start = self.consume_keyword(Keyword::Def, "expected 'def'")?;
        let (name, _) = self.expect_identifier("expected function name")?;
        let type_params = self.parse_type_params()?;
        let params = self.parse_param_list()?;
        let return_type = self.parse_optional_return_type();
        let body = self.parse_callable_body()?;
        let end = body.span();
        Some(FunctionDecl {
            visibility,
            name,
            type_params,
            params,
            return_type,
            body,
            span: start.cover(end),
        })
    }

    fn parse_type_decl(&mut self, visibility: Visibility) -> Option<TypeDecl> {
        let (kind, start) = match self.current_kind() {
            TokenKind::Keyword(Keyword::Class) => {
                let span = self.current_span();
                self.advance();
                (TypeKind::Class, span)
            }
            TokenKind::Keyword(Keyword::Record) => {
                let span = self.current_span();
                self.advance();
                (TypeKind::Record, span)
            }
            TokenKind::Keyword(Keyword::Object) => {
                let span = self.current_span();
                self.advance();
                (TypeKind::Object, span)
            }
            TokenKind::Keyword(Keyword::Interface) => {
                let span = self.current_span();
                self.advance();
                (TypeKind::Interface, span)
            }
            TokenKind::Keyword(Keyword::Enum) => {
                let span = self.current_span();
                self.advance();
                (TypeKind::Enum, span)
            }
            _ => {
                self.error_at_current(
                    "expected_type_decl",
                    "expected class, record, object, interface, or enum",
                );
                return None;
            }
        };

        let (name, _) = self.expect_identifier("expected type name")?;
        let type_params = self.parse_type_params()?;
        let with_bounds = if self.match_keyword(Keyword::With) {
            self.parse_type_ref_list()?
        } else {
            Vec::new()
        };
        self.skip_newlines();
        self.consume(TokenKind::LBrace, "expected '{' after type declaration")?;
        self.skip_newlines();

        let mut members = Vec::new();
        while !self.at(TokenKind::RBrace) && !self.at(TokenKind::Eof) {
            self.skip_newlines();
            if self.at(TokenKind::RBrace) {
                break;
            }

            if kind == TypeKind::Enum && self.match_keyword(Keyword::Case) {
                if let Some(case_decl) = self.parse_enum_case(self.previous_span()) {
                    members.push(TypeMember::Case(case_decl));
                } else {
                    self.synchronize_member();
                }
                self.skip_newlines();
                continue;
            }

            let member_visibility = self.parse_visibility();
            match self.current_kind() {
                TokenKind::Keyword(Keyword::Def) => {
                    let method =
                        self.parse_method_decl(member_visibility, kind == TypeKind::Interface)?;
                    members.push(TypeMember::Method(method));
                }
                _ => {
                    let field = self.parse_field_decl(member_visibility)?;
                    members.push(TypeMember::Field(field));
                }
            }
            self.skip_newlines();
        }

        let end = self.consume(TokenKind::RBrace, "expected '}' after type body")?;
        Some(TypeDecl {
            visibility,
            kind,
            name,
            type_params,
            with_bounds,
            members,
            span: start.cover(end),
        })
    }

    fn parse_enum_case(&mut self, case_span: Span) -> Option<EnumCaseDecl> {
        let (name, name_span) = self.expect_identifier("expected enum case name")?;
        let mut fields = Vec::new();
        self.skip_newlines();
        if self.match_token(TokenKind::LBrace) {
            self.skip_newlines();
            while !self.at(TokenKind::RBrace) && !self.at(TokenKind::Eof) {
                let field_visibility = self.parse_visibility();
                let field = self.parse_field_decl(field_visibility)?;
                fields.push(field);
                self.skip_newlines();
            }
            let end = self.consume(TokenKind::RBrace, "expected '}' after enum case body")?;
            return Some(EnumCaseDecl {
                name,
                fields,
                span: case_span.cover(end),
            });
        }
        Some(EnumCaseDecl {
            name,
            fields,
            span: case_span.cover(name_span),
        })
    }

    fn parse_impl_block(&mut self) -> Option<ImplBlock> {
        let start = self.consume_keyword(Keyword::Impl, "expected 'impl'")?;
        let target = self.parse_type_ref()?;
        self.skip_newlines();
        self.consume(TokenKind::LBrace, "expected '{' after impl target")?;
        self.skip_newlines();

        let mut methods = Vec::new();
        while !self.at(TokenKind::RBrace) && !self.at(TokenKind::Eof) {
            self.skip_newlines();
            if self.at(TokenKind::RBrace) {
                break;
            }
            let visibility = self.parse_visibility();
            let method = self.parse_method_decl(visibility, false)?;
            methods.push(method);
            self.skip_newlines();
        }
        let end = self.consume(TokenKind::RBrace, "expected '}' after impl body")?;
        Some(ImplBlock {
            target,
            methods,
            span: start.cover(end),
        })
    }

    fn parse_method_decl(
        &mut self,
        visibility: Visibility,
        allow_signature_only: bool,
    ) -> Option<MethodDecl> {
        let start = self.consume_keyword(Keyword::Def, "expected 'def'")?;
        let (name, _) = self.expect_identifier("expected method name")?;
        let type_params = self.parse_type_params()?;
        let params = self.parse_param_list()?;
        let return_type = self.parse_optional_return_type();
        let body = if self.at(TokenKind::LBrace) || self.at(TokenKind::Eq) {
            Some(self.parse_callable_body()?)
        } else if allow_signature_only {
            None
        } else {
            self.error_at_current("expected_method_body", "expected method body");
            return None;
        };
        let end = body
            .as_ref()
            .map(CallableBody::span)
            .or_else(|| return_type.as_ref().map(TypeRef::span))
            .unwrap_or(start);
        Some(MethodDecl {
            visibility,
            name,
            type_params,
            params,
            return_type,
            body,
            span: start.cover(end),
        })
    }

    fn parse_field_decl(&mut self, visibility: Visibility) -> Option<FieldDecl> {
        let start = self.current_span();
        let mutable = self.match_keyword(Keyword::Var);
        let (name, _) = self.expect_identifier("expected field name")?;

        let ty = if self.can_start_type_ref() {
            Some(self.parse_type_ref()?)
        } else {
            None
        };

        let (initializer, deferred, end) =
            if self.match_token(TokenKind::Eq) || self.match_token(TokenKind::ColonAssign) {
                let assign_span = self.previous_span();
                if self.match_token(TokenKind::Question) {
                    (None, true, self.previous_span())
                } else {
                    let expr = self.parse_expr()?;
                    let end = expr.span();
                    (Some(expr), false, assign_span.cover(end))
                }
            } else {
                (None, false, ty.as_ref().map(TypeRef::span).unwrap_or(start))
            };

        Some(FieldDecl {
            visibility,
            mutable,
            name,
            ty,
            initializer,
            deferred,
            span: start.cover(end),
        })
    }

    fn parse_callable_body(&mut self) -> Option<CallableBody> {
        self.skip_newlines();
        if self.at(TokenKind::LBrace) {
            return self.parse_block().map(CallableBody::Block);
        }
        self.consume(TokenKind::Eq, "expected '=' or '{' before callable body")?;
        let expr = self.parse_expr()?;
        Some(CallableBody::Expr(expr))
    }

    fn parse_block(&mut self) -> Option<Block> {
        let start = self.consume(TokenKind::LBrace, "expected '{'")?;
        self.skip_newlines();
        let mut statements = Vec::new();
        while !self.at(TokenKind::RBrace) && !self.at(TokenKind::Eof) {
            if let Some(stmt) = self.parse_stmt() {
                statements.push(stmt);
            } else {
                self.synchronize_stmt();
            }
            self.skip_newlines();
        }
        let end = self.consume(TokenKind::RBrace, "expected '}' after block")?;
        Some(Block {
            statements,
            span: start.cover(end),
        })
    }

    fn parse_stmt(&mut self) -> Option<Stmt> {
        self.skip_newlines();
        match self.current_kind() {
            TokenKind::Keyword(Keyword::Def) => {
                let function = self.parse_function_decl(Visibility::Default)?;
                Some(Stmt::LocalFunction(function))
            }
            TokenKind::Keyword(Keyword::Var) => {
                let stmt = self.parse_binding_stmt_after_var()?;
                Some(Stmt::Binding(stmt))
            }
            TokenKind::Keyword(Keyword::If) => self.parse_if_stmt().map(Stmt::If),
            TokenKind::Keyword(Keyword::While) => self.parse_while_stmt().map(Stmt::While),
            TokenKind::Keyword(Keyword::For) => self.parse_for_stmt().map(Stmt::For),
            TokenKind::Keyword(Keyword::Return) => self.parse_return_stmt().map(Stmt::Return),
            TokenKind::Keyword(Keyword::Break) => self.parse_break_stmt().map(Stmt::Break),
            _ => {
                if let Some(binding) = self.try_parse_binding_stmt() {
                    return Some(Stmt::Binding(binding));
                }
                if let Some(assignment) = self.try_parse_assignment_stmt() {
                    return Some(Stmt::Assignment(assignment));
                }
                let expr = self.parse_expr()?;
                let span = expr.span();
                Some(Stmt::Expr(ExprStmt { expr, span }))
            }
        }
    }

    fn parse_binding_stmt_after_var(&mut self) -> Option<BindingStmt> {
        let start = self.consume_keyword(Keyword::Var, "expected 'var'")?;
        let mut bindings = vec![self.parse_binding(true)?];
        while self.match_token(TokenKind::Comma) {
            bindings.push(self.parse_binding(true)?);
        }
        self.consume(TokenKind::Eq, "expected '=' after bindings")?;
        let values = self.parse_expr_list()?;
        let end = values.last().map(Expr::span).unwrap_or(start);
        Some(BindingStmt {
            bindings,
            values,
            span: start.cover(end),
        })
    }

    fn try_parse_binding_stmt(&mut self) -> Option<BindingStmt> {
        let checkpoint = self.checkpoint();
        let Some(first) = self.parse_binding(false) else {
            self.restore(checkpoint);
            return None;
        };
        let mut bindings = vec![first];
        while self.match_token(TokenKind::Comma) {
            let Some(binding) = self.parse_binding(false) else {
                self.restore(checkpoint);
                return None;
            };
            bindings.push(binding);
        }
        if !self.match_token(TokenKind::Eq) {
            self.restore(checkpoint);
            return None;
        }
        let Some(values) = self.parse_expr_list() else {
            self.restore(checkpoint);
            return None;
        };
        let start = bindings[0].span;
        let end = values.last().map(Expr::span).unwrap_or(start);
        Some(BindingStmt {
            bindings,
            values,
            span: start.cover(end),
        })
    }

    fn parse_binding(&mut self, mutable: bool) -> Option<Binding> {
        let (name, start) = self.expect_identifier("expected binding name")?;
        let ty = if self.can_start_type_ref() {
            Some(self.parse_type_ref()?)
        } else {
            None
        };
        let span = ty.as_ref().map(TypeRef::span).unwrap_or(start);
        Some(Binding {
            name,
            ty,
            mutable,
            deferred: false,
            span: start.cover(span),
        })
    }

    fn try_parse_assignment_stmt(&mut self) -> Option<AssignmentStmt> {
        let checkpoint = self.checkpoint();
        let Some(targets) = self.parse_expr_list() else {
            self.restore(checkpoint);
            return None;
        };
        let operator =
            if self.match_token(TokenKind::Eq) || self.match_token(TokenKind::ColonAssign) {
                AssignOp::Reassign
            } else if self.match_token(TokenKind::PlusEq) {
                AssignOp::AddAssign
            } else if self.match_token(TokenKind::MinusEq) {
                AssignOp::SubAssign
            } else if self.match_token(TokenKind::StarEq) {
                AssignOp::MulAssign
            } else if self.match_token(TokenKind::SlashEq) {
                AssignOp::DivAssign
            } else {
                self.restore(checkpoint);
                return None;
            };
        let Some(values) = self.parse_expr_list() else {
            self.restore(checkpoint);
            return None;
        };
        let start = targets
            .first()
            .map(Expr::span)
            .unwrap_or(self.previous_span());
        let end = values
            .last()
            .map(Expr::span)
            .unwrap_or(self.previous_span());
        Some(AssignmentStmt {
            targets,
            operator,
            values,
            span: start.cover(end),
        })
    }

    fn parse_if_stmt(&mut self) -> Option<IfStmt> {
        let start = self.consume_keyword(Keyword::If, "expected 'if'")?;
        let condition = self.parse_expr()?;
        let then_block = self.parse_block()?;
        let else_branch = if self.match_keyword(Keyword::Else) {
            self.skip_newlines();
            if self.at_keyword(Keyword::If) {
                Some(ElseBranch::If(Box::new(self.parse_if_stmt()?)))
            } else {
                Some(ElseBranch::Block(self.parse_block()?))
            }
        } else {
            None
        };
        let end = else_branch
            .as_ref()
            .map(|branch| match branch {
                ElseBranch::If(if_stmt) => if_stmt.span,
                ElseBranch::Block(block) => block.span,
            })
            .unwrap_or(then_block.span);
        Some(IfStmt {
            condition,
            then_block,
            else_branch,
            span: start.cover(end),
        })
    }

    fn parse_while_stmt(&mut self) -> Option<WhileStmt> {
        let start = self.consume_keyword(Keyword::While, "expected 'while'")?;
        let condition = self.parse_expr()?;
        let body = self.parse_block()?;
        Some(WhileStmt {
            condition,
            body: body.clone(),
            span: start.cover(body.span),
        })
    }

    fn parse_for_stmt(&mut self) -> Option<ForStmt> {
        let start = self.consume_keyword(Keyword::For, "expected 'for'")?;
        let mut bindings = vec![self.parse_binding(false)?];
        while self.match_token(TokenKind::Comma) {
            bindings.push(self.parse_binding(false)?);
        }
        self.consume(TokenKind::LeftArrow, "expected '<-' in for loop")?;
        let iterable = self.parse_expr()?;
        let body = self.parse_block()?;
        Some(ForStmt {
            bindings,
            iterable,
            body: body.clone(),
            span: start.cover(body.span),
        })
    }

    fn parse_return_stmt(&mut self) -> Option<ReturnStmt> {
        let start = self.consume_keyword(Keyword::Return, "expected 'return'")?;
        if self.at(TokenKind::Newline) || self.at(TokenKind::RBrace) || self.at(TokenKind::Eof) {
            return Some(ReturnStmt {
                value: None,
                span: start,
            });
        }
        let value = self.parse_expr()?;
        let end = value.span();
        Some(ReturnStmt {
            value: Some(value),
            span: start.cover(end),
        })
    }

    fn parse_break_stmt(&mut self) -> Option<BreakStmt> {
        let span = self.consume_keyword(Keyword::Break, "expected 'break'")?;
        Some(BreakStmt { span })
    }

    fn parse_expr(&mut self) -> Option<Expr> {
        self.skip_newlines();
        if let Some(lambda) = self.try_parse_lambda_expr() {
            return Some(lambda);
        }
        if self.match_keyword(Keyword::If) {
            return self.parse_if_expr(self.previous_span());
        }
        self.parse_colon_expr()
    }

    fn try_parse_lambda_expr(&mut self) -> Option<Expr> {
        let checkpoint = self.checkpoint();
        if self.at(TokenKind::Identifier) {
            if let Some(param) = self.parse_lambda_param() {
                if self.match_token(TokenKind::Arrow) {
                    let start = param.span;
                    let body = match self.parse_lambda_body() {
                        Some(body) => body,
                        None => {
                            self.restore(checkpoint);
                            return None;
                        }
                    };
                    let end = body.span();
                    return Some(Expr::Lambda {
                        params: vec![param],
                        body,
                        span: start.cover(end),
                    });
                }
            }
            self.restore(checkpoint);
            return None;
        }
        if !self.match_token(TokenKind::LParen) {
            return None;
        }
        let start = self.previous_span();
        let mut params = Vec::new();
        self.skip_newlines();
        if !self.at(TokenKind::RParen) {
            let Some(param) = self.parse_lambda_param() else {
                self.restore(checkpoint);
                return None;
            };
            params.push(param);
            while self.match_token(TokenKind::Comma) {
                let Some(param) = self.parse_lambda_param() else {
                    self.restore(checkpoint);
                    return None;
                };
                params.push(param);
            }
        }
        self.skip_newlines();
        if self
            .consume(TokenKind::RParen, "expected ')' after lambda parameters")
            .is_none()
        {
            self.restore(checkpoint);
            return None;
        }
        if !self.match_token(TokenKind::Arrow) {
            self.restore(checkpoint);
            return None;
        }
        let Some(body) = self.parse_lambda_body() else {
            self.restore(checkpoint);
            return None;
        };
        let end = body.span();
        Some(Expr::Lambda {
            params,
            body,
            span: start.cover(end),
        })
    }

    fn parse_lambda_param(&mut self) -> Option<LambdaParam> {
        let (name, start) = self.expect_identifier("expected lambda parameter")?;
        let ty = if self.can_start_type_ref() {
            Some(self.parse_type_ref()?)
        } else {
            None
        };
        let end = ty.as_ref().map(TypeRef::span).unwrap_or(start);
        Some(LambdaParam {
            name,
            ty,
            span: start.cover(end),
        })
    }

    fn parse_lambda_body(&mut self) -> Option<LambdaBody> {
        if self.at(TokenKind::LBrace) {
            return self.parse_block().map(LambdaBody::Block);
        }
        self.parse_expr()
            .map(|expr| LambdaBody::Expr(Box::new(expr)))
    }

    fn parse_if_expr(&mut self, start: Span) -> Option<Expr> {
        let condition = self.parse_expr()?;
        let then_block = self.parse_block()?;
        self.consume_keyword(Keyword::Else, "expected 'else' in if expression")?;
        self.skip_newlines();
        let else_branch = if self.at_keyword(Keyword::If) {
            let else_if = self.parse_expr()?;
            ElseExprBranch::If(Box::new(else_if))
        } else {
            ElseExprBranch::Block(self.parse_block()?)
        };
        let end = match &else_branch {
            ElseExprBranch::If(expr) => expr.span(),
            ElseExprBranch::Block(block) => block.span,
        };
        Some(Expr::If {
            condition: Box::new(condition),
            then_block,
            else_branch: Box::new(else_branch),
            span: start.cover(end),
        })
    }

    fn parse_colon_expr(&mut self) -> Option<Expr> {
        let mut expr = self.parse_or_expr()?;
        while self.match_token(TokenKind::Colon) {
            let right = self.parse_or_expr()?;
            let span = expr.span().cover(right.span());
            expr = Expr::Binary {
                left: Box::new(expr),
                op: BinaryOp::Colon,
                right: Box::new(right),
                span,
            };
        }
        Some(expr)
    }

    fn parse_or_expr(&mut self) -> Option<Expr> {
        self.parse_left_assoc(
            |parser| parser.parse_and_expr(),
            &[(TokenKind::OrOr, BinaryOp::Or)],
        )
    }

    fn parse_and_expr(&mut self) -> Option<Expr> {
        self.parse_left_assoc(
            |parser| parser.parse_equality_expr(),
            &[(TokenKind::AndAnd, BinaryOp::And)],
        )
    }

    fn parse_equality_expr(&mut self) -> Option<Expr> {
        self.parse_left_assoc(
            |parser| parser.parse_comparison_expr(),
            &[
                (TokenKind::EqEq, BinaryOp::Eq),
                (TokenKind::NotEq, BinaryOp::NotEq),
            ],
        )
    }

    fn parse_comparison_expr(&mut self) -> Option<Expr> {
        self.parse_left_assoc(
            |parser| parser.parse_term_expr(),
            &[
                (TokenKind::Less, BinaryOp::Less),
                (TokenKind::LessEq, BinaryOp::LessEq),
                (TokenKind::Greater, BinaryOp::Greater),
                (TokenKind::GreaterEq, BinaryOp::GreaterEq),
            ],
        )
    }

    fn parse_term_expr(&mut self) -> Option<Expr> {
        self.parse_left_assoc(
            |parser| parser.parse_factor_expr(),
            &[
                (TokenKind::Plus, BinaryOp::Add),
                (TokenKind::Minus, BinaryOp::Sub),
            ],
        )
    }

    fn parse_factor_expr(&mut self) -> Option<Expr> {
        self.parse_left_assoc(
            |parser| parser.parse_unary_expr(),
            &[
                (TokenKind::Star, BinaryOp::Mul),
                (TokenKind::Slash, BinaryOp::Div),
                (TokenKind::Percent, BinaryOp::Mod),
            ],
        )
    }

    fn parse_left_assoc<F>(
        &mut self,
        mut parse_operand: F,
        operators: &[(TokenKind, BinaryOp)],
    ) -> Option<Expr>
    where
        F: FnMut(&mut Self) -> Option<Expr>,
    {
        let mut expr = parse_operand(self)?;
        loop {
            let mut matched = None;
            for (kind, op) in operators {
                if self.match_token(*kind) {
                    matched = Some(*op);
                    break;
                }
            }
            let Some(op) = matched else {
                break;
            };
            let right = parse_operand(self)?;
            let span = expr.span().cover(right.span());
            expr = Expr::Binary {
                left: Box::new(expr),
                op,
                right: Box::new(right),
                span,
            };
        }
        Some(expr)
    }

    fn parse_unary_expr(&mut self) -> Option<Expr> {
        if self.match_token(TokenKind::Bang) {
            let start = self.previous_span();
            let expr = self.parse_unary_expr()?;
            let span = start.cover(expr.span());
            return Some(Expr::Unary {
                op: UnaryOp::Not,
                expr: Box::new(expr),
                span,
            });
        }
        if self.match_token(TokenKind::Minus) {
            let start = self.previous_span();
            let expr = self.parse_unary_expr()?;
            let span = start.cover(expr.span());
            return Some(Expr::Unary {
                op: UnaryOp::Neg,
                expr: Box::new(expr),
                span,
            });
        }
        self.parse_postfix_expr()
    }

    fn parse_postfix_expr(&mut self) -> Option<Expr> {
        let mut expr = self.parse_primary_expr()?;
        loop {
            self.skip_newlines();
            if self.match_token(TokenKind::LParen) {
                let start = expr.span();
                let args = self.parse_call_args()?;
                let end = self.consume(TokenKind::RParen, "expected ')' after arguments")?;
                expr = Expr::Call {
                    callee: Box::new(expr),
                    args,
                    span: start.cover(end),
                };
                continue;
            }
            if self.match_token(TokenKind::Dot) {
                let (name, end) = self.expect_identifier("expected member name after '.'")?;
                let start = expr.span();
                expr = Expr::Member {
                    receiver: Box::new(expr),
                    name,
                    span: start.cover(end),
                };
                continue;
            }
            if self.match_token(TokenKind::LBracket) {
                let start = expr.span();
                let index = self.parse_expr()?;
                let end = self.consume(TokenKind::RBracket, "expected ']' after index")?;
                expr = Expr::Index {
                    receiver: Box::new(expr),
                    index: Box::new(index),
                    span: start.cover(end),
                };
                continue;
            }
            break;
        }
        Some(expr)
    }

    fn parse_call_args(&mut self) -> Option<Vec<CallArg>> {
        let mut args = Vec::new();
        self.skip_newlines();
        if self.at(TokenKind::RParen) {
            return Some(args);
        }
        loop {
            let start = self.current_span();
            if self.at(TokenKind::Identifier) && self.at_next(TokenKind::Eq) {
                let (name, name_span) = self.expect_identifier("expected named argument")?;
                self.consume(TokenKind::Eq, "expected '=' after argument name")?;
                let value = self.parse_expr()?;
                let span = name_span.cover(value.span());
                args.push(CallArg {
                    name: Some(name),
                    value,
                    span,
                });
            } else {
                let value = self.parse_expr()?;
                args.push(CallArg {
                    name: None,
                    span: value.span(),
                    value,
                });
            }
            self.skip_newlines();
            if !self.match_token(TokenKind::Comma) {
                break;
            }
            self.skip_newlines();
            if self.at(TokenKind::RParen) {
                break;
            }
            let _ = start;
        }
        Some(args)
    }

    fn parse_primary_expr(&mut self) -> Option<Expr> {
        self.skip_newlines();
        match self.current_kind() {
            TokenKind::Identifier => {
                let token = self.current().clone();
                self.advance();
                Some(Expr::Identifier {
                    name: token.lexeme,
                    span: token.span,
                })
            }
            TokenKind::Integer => {
                let token = self.current().clone();
                self.advance();
                Some(Expr::Integer {
                    raw: token.lexeme,
                    span: token.span,
                })
            }
            TokenKind::String => {
                let token = self.current().clone();
                self.advance();
                Some(Expr::String {
                    raw: token.lexeme,
                    span: token.span,
                })
            }
            TokenKind::Keyword(Keyword::True) => {
                let span = self.current_span();
                self.advance();
                Some(Expr::Bool { value: true, span })
            }
            TokenKind::Keyword(Keyword::False) => {
                let span = self.current_span();
                self.advance();
                Some(Expr::Bool { value: false, span })
            }
            TokenKind::LBracket => self.parse_list_literal(),
            TokenKind::LParen => self.parse_group_or_tuple_expr(),
            _ => {
                self.error_at_current("expected_expression", "expected expression");
                None
            }
        }
    }

    fn parse_list_literal(&mut self) -> Option<Expr> {
        let start = self.consume(TokenKind::LBracket, "expected '['")?;
        self.skip_newlines();
        let mut items = Vec::new();
        if !self.at(TokenKind::RBracket) {
            items.push(self.parse_expr()?);
            while self.match_token(TokenKind::Comma) {
                self.skip_newlines();
                if self.at(TokenKind::RBracket) {
                    break;
                }
                items.push(self.parse_expr()?);
            }
        }
        let end = self.consume(TokenKind::RBracket, "expected ']' after list literal")?;
        Some(Expr::ListLiteral {
            items,
            span: start.cover(end),
        })
    }

    fn parse_group_or_tuple_expr(&mut self) -> Option<Expr> {
        let start = self.consume(TokenKind::LParen, "expected '('")?;
        self.skip_newlines();
        if self.match_token(TokenKind::RParen) {
            let end = self.previous_span();
            return Some(Expr::Unit {
                span: start.cover(end),
            });
        }

        let first = self.parse_expr()?;
        self.skip_newlines();
        if self.match_token(TokenKind::Comma) {
            let mut items = vec![first];
            self.skip_newlines();
            if !self.at(TokenKind::RParen) {
                items.push(self.parse_expr()?);
                while self.match_token(TokenKind::Comma) {
                    self.skip_newlines();
                    if self.at(TokenKind::RParen) {
                        break;
                    }
                    items.push(self.parse_expr()?);
                }
            }
            let end = self.consume(TokenKind::RParen, "expected ')' after tuple literal")?;
            return Some(Expr::TupleLiteral {
                items,
                span: start.cover(end),
            });
        }
        let end = self.consume(TokenKind::RParen, "expected ')' after grouped expression")?;
        let mut expr = first;
        match &mut expr {
            Expr::Identifier { span, .. }
            | Expr::Integer { span, .. }
            | Expr::String { span, .. }
            | Expr::Bool { span, .. }
            | Expr::Unit { span }
            | Expr::ListLiteral { span, .. }
            | Expr::TupleLiteral { span, .. }
            | Expr::Call { span, .. }
            | Expr::Member { span, .. }
            | Expr::Index { span, .. }
            | Expr::Unary { span, .. }
            | Expr::Binary { span, .. }
            | Expr::If { span, .. }
            | Expr::Lambda { span, .. } => *span = start.cover(end),
        }
        Some(expr)
    }

    fn parse_expr_list(&mut self) -> Option<Vec<Expr>> {
        let mut exprs = vec![self.parse_expr()?];
        while self.match_token(TokenKind::Comma) {
            exprs.push(self.parse_expr()?);
        }
        Some(exprs)
    }

    fn parse_type_params(&mut self) -> Option<Vec<TypeParam>> {
        if !self.match_token(TokenKind::LBracket) {
            return Some(Vec::new());
        }
        let mut params = Vec::new();
        self.skip_newlines();
        if !self.at(TokenKind::RBracket) {
            loop {
                let (name, start) = self.expect_identifier("expected type parameter name")?;
                let bounds = if self.match_keyword(Keyword::With) {
                    self.parse_type_ref_list()?
                } else {
                    Vec::new()
                };
                let end = bounds.last().map(TypeRef::span).unwrap_or(start);
                params.push(TypeParam {
                    name,
                    bounds,
                    span: start.cover(end),
                });
                self.skip_newlines();
                if !self.match_token(TokenKind::Comma) {
                    break;
                }
                self.skip_newlines();
            }
        }
        self.consume(TokenKind::RBracket, "expected ']' after type parameters")?;
        Some(params)
    }

    fn parse_param_list(&mut self) -> Option<Vec<Param>> {
        self.consume(TokenKind::LParen, "expected '(' before parameter list")?;
        let mut params = Vec::new();
        self.skip_newlines();
        if !self.at(TokenKind::RParen) {
            loop {
                let (name, start) = self.expect_identifier("expected parameter name")?;
                let ty = if self.can_start_type_ref() {
                    Some(self.parse_type_ref()?)
                } else {
                    None
                };
                let end = ty.as_ref().map(TypeRef::span).unwrap_or(start);
                params.push(Param {
                    name,
                    ty,
                    variadic: false,
                    span: start.cover(end),
                });
                self.skip_newlines();
                if !self.match_token(TokenKind::Comma) {
                    break;
                }
                self.skip_newlines();
            }
        }
        self.consume(TokenKind::RParen, "expected ')' after parameters")?;
        Some(params)
    }

    fn parse_optional_return_type(&mut self) -> Option<TypeRef> {
        self.skip_newlines();
        if self.can_start_type_ref() {
            self.parse_type_ref()
        } else {
            None
        }
    }

    fn parse_type_ref_list(&mut self) -> Option<Vec<TypeRef>> {
        let mut refs = vec![self.parse_type_ref()?];
        while self.match_token(TokenKind::Comma) {
            refs.push(self.parse_type_ref()?);
        }
        Some(refs)
    }

    fn parse_type_ref(&mut self) -> Option<TypeRef> {
        let left = self.parse_primary_type_ref()?;
        if self.match_token(TokenKind::Arrow) {
            let ret = self.parse_type_ref()?;
            let params = match left {
                TypeRef::Tuple { fields, .. } => fields.into_iter().map(|field| field.ty).collect(),
                other => vec![other],
            };
            let span = params
                .first()
                .map(TypeRef::span)
                .unwrap_or(ret.span())
                .cover(ret.span());
            return Some(TypeRef::Function {
                params,
                ret: Box::new(ret),
                span,
            });
        }
        Some(left)
    }

    fn parse_primary_type_ref(&mut self) -> Option<TypeRef> {
        self.skip_newlines();
        if self.match_token(TokenKind::LParen) {
            let start = self.previous_span();
            self.skip_newlines();
            let mut fields = Vec::new();
            if !self.at(TokenKind::RParen) {
                fields.push(self.parse_tuple_type_field()?);
                while self.match_token(TokenKind::Comma) {
                    self.skip_newlines();
                    if self.at(TokenKind::RParen) {
                        break;
                    }
                    fields.push(self.parse_tuple_type_field()?);
                }
            }
            let end = self.consume(TokenKind::RParen, "expected ')' after tuple type")?;
            if fields.len() == 1 && fields[0].name.is_none() {
                return Some(fields.into_iter().next()?.ty);
            }
            return Some(TypeRef::Tuple {
                fields,
                span: start.cover(end),
            });
        }

        let (name, start) = self.expect_identifier("expected type name")?;
        let mut args = Vec::new();
        if self.match_token(TokenKind::LBracket) {
            self.skip_newlines();
            if !self.at(TokenKind::RBracket) {
                args.push(self.parse_type_ref()?);
                while self.match_token(TokenKind::Comma) {
                    args.push(self.parse_type_ref()?);
                }
            }
            self.consume(TokenKind::RBracket, "expected ']' after type arguments")?;
        }
        let end = if let Some(last) = args.last() {
            last.span()
        } else {
            start
        };
        Some(TypeRef::Named {
            name,
            args,
            span: start.cover(end),
        })
    }

    fn parse_tuple_type_field(&mut self) -> Option<TupleTypeField> {
        let checkpoint = self.checkpoint();
        if self.at(TokenKind::Identifier) && self.at_next(TokenKind::Identifier) {
            let (name, name_span) = self.expect_identifier("expected tuple field name")?;
            let ty = self.parse_type_ref()?;
            let span = name_span.cover(ty.span());
            return Some(TupleTypeField {
                name: Some(name),
                ty,
                span,
            });
        }
        self.restore(checkpoint);
        let ty = self.parse_type_ref()?;
        let span = ty.span();
        Some(TupleTypeField {
            name: None,
            ty,
            span,
        })
    }

    fn can_start_type_ref(&self) -> bool {
        matches!(
            self.current_kind(),
            TokenKind::Identifier | TokenKind::LParen
        )
    }

    fn parse_path_string(&mut self) -> Option<String> {
        let (first, _) = self.expect_identifier("expected path segment")?;
        let mut path = first;
        while self.match_token(TokenKind::Slash) {
            let (segment, _) = self.expect_identifier("expected path segment after '/'")?;
            path.push('/');
            path.push_str(&segment);
        }
        Some(path)
    }

    fn synchronize_item(&mut self) {
        while !self.at(TokenKind::Eof) {
            if self.at(TokenKind::Newline) {
                self.advance();
                return;
            }
            match self.current_kind() {
                TokenKind::Keyword(Keyword::Def)
                | TokenKind::Keyword(Keyword::Class)
                | TokenKind::Keyword(Keyword::Record)
                | TokenKind::Keyword(Keyword::Object)
                | TokenKind::Keyword(Keyword::Interface)
                | TokenKind::Keyword(Keyword::Enum)
                | TokenKind::Keyword(Keyword::Impl) => return,
                _ => self.advance(),
            }
        }
    }

    fn synchronize_member(&mut self) {
        while !self.at(TokenKind::Eof) && !self.at(TokenKind::RBrace) {
            if self.at(TokenKind::Newline) {
                self.advance();
                return;
            }
            self.advance();
        }
    }

    fn synchronize_stmt(&mut self) {
        while !self.at(TokenKind::Eof) && !self.at(TokenKind::RBrace) {
            if self.at(TokenKind::Newline) {
                self.advance();
                return;
            }
            self.advance();
        }
    }

    fn checkpoint(&self) -> Checkpoint {
        Checkpoint {
            index: self.index,
            diagnostics_len: self.diagnostics.len(),
        }
    }

    fn restore(&mut self, checkpoint: Checkpoint) {
        self.index = checkpoint.index;
        self.diagnostics.truncate(checkpoint.diagnostics_len);
    }

    fn skip_newlines(&mut self) {
        while self.match_token(TokenKind::Newline) {}
    }

    fn consume(&mut self, kind: TokenKind, message: &'static str) -> Option<Span> {
        if self.match_token(kind) {
            Some(self.previous_span())
        } else {
            self.error_at_current("unexpected_token", message);
            None
        }
    }

    fn consume_keyword(&mut self, keyword: Keyword, message: &'static str) -> Option<Span> {
        if self.match_keyword(keyword) {
            Some(self.previous_span())
        } else {
            self.error_at_current("unexpected_token", message);
            None
        }
    }

    fn expect_identifier(&mut self, message: &'static str) -> Option<(String, Span)> {
        if self.at(TokenKind::Identifier) {
            let token = self.current().clone();
            self.advance();
            Some((token.lexeme, token.span))
        } else {
            self.error_at_current("expected_identifier", message);
            None
        }
    }

    fn match_keyword(&mut self, keyword: Keyword) -> bool {
        if self.at_keyword(keyword) {
            self.advance();
            true
        } else {
            false
        }
    }

    fn at_keyword(&self, keyword: Keyword) -> bool {
        matches!(self.current_kind(), TokenKind::Keyword(k) if k == keyword)
    }

    fn match_token(&mut self, kind: TokenKind) -> bool {
        if self.at(kind) {
            self.advance();
            true
        } else {
            false
        }
    }

    fn at(&self, kind: TokenKind) -> bool {
        self.current_kind() == kind
    }

    fn at_next(&self, kind: TokenKind) -> bool {
        self.tokens
            .get(self.index + 1)
            .map(|token| token.kind == kind)
            .unwrap_or(false)
    }

    fn current(&self) -> &Token {
        &self.tokens[self.index.min(self.tokens.len().saturating_sub(1))]
    }

    fn current_kind(&self) -> TokenKind {
        self.current().kind
    }

    fn current_span(&self) -> Span {
        self.current().span
    }

    fn previous_span(&self) -> Span {
        self.tokens
            .get(self.index.saturating_sub(1))
            .map(|token| token.span)
            .unwrap_or_else(|| self.current_span())
    }

    fn last_non_newline_span(&self, fallback: Span) -> Span {
        for token in self.tokens[..self.index].iter().rev() {
            if token.kind != TokenKind::Newline {
                return token.span;
            }
        }
        fallback
    }

    fn advance(&mut self) {
        if !self.at(TokenKind::Eof) {
            self.index += 1;
        }
    }

    fn error_at_current(&mut self, code: &'static str, message: impl Into<String>) {
        let span = self.current_span();
        self.diagnostics
            .push(Diagnostic::error(code, message, span));
    }
}

impl CallableBody {
    fn span(&self) -> Span {
        match self {
            CallableBody::Block(block) => block.span,
            CallableBody::Expr(expr) => expr.span(),
        }
    }
}

impl LambdaBody {
    fn span(&self) -> Span {
        match self {
            LambdaBody::Expr(expr) => expr.span(),
            LambdaBody::Block(block) => block.span,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{lexer::lex, source::SourceFile};

    fn parse(src: &str) -> ParseResult {
        let file = SourceFile::new("test.lum", src);
        let lexed = lex(&file);
        assert!(
            lexed.diagnostics.is_empty(),
            "lexer diagnostics: {:#?}",
            lexed.diagnostics
        );
        parse_program(&lexed.tokens)
    }

    #[test]
    fn parses_function_with_range_loop() {
        let result = parse(
            r#"
def run(limit Int) Int {
    var total Int = 0
    for i <- Range(0, limit) {
        total += i
    }
    return total
}
"#,
        );
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
        let program = result.program.expect("program");
        assert_eq!(program.items.len(), 1);
        match &program.items[0] {
            Item::Function(function) => {
                assert_eq!(function.name, "run");
                match &function.body {
                    CallableBody::Block(block) => {
                        assert_eq!(block.statements.len(), 3);
                    }
                    other => panic!("expected block body, got {other:#?}"),
                }
            }
            other => panic!("expected function, got {other:#?}"),
        }
    }

    #[test]
    fn parses_class_and_impl() {
        let result = parse(
            r#"
class Counter {
    hidden var count Int
}

impl Counter {
    def init(count Int) {
        this.count = count
    }

    def bump(delta Int) Int = this.count + delta
}
"#,
        );
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
        let program = result.program.expect("program");
        assert_eq!(program.items.len(), 2);
        assert!(matches!(program.items[0], Item::Type(_)));
        assert!(matches!(program.items[1], Item::Impl(_)));
    }

    #[test]
    fn parses_single_expression_function_body() {
        let result = parse("def zero() Int = 0\n");
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
        let program = result.program.expect("program");
        match &program.items[0] {
            Item::Function(function) => match &function.body {
                CallableBody::Expr(Expr::Integer { raw, .. }) => assert_eq!(raw, "0"),
                other => panic!("expected integer expr body, got {other:#?}"),
            },
            other => panic!("expected function, got {other:#?}"),
        }
    }

    #[test]
    fn parses_if_expression_and_calls() {
        let result = parse(
            r#"
def run(flag Bool) Int {
    value Int = if flag {
        foo(1)
    } else {
        bar(2)
    }
    return value
}
"#,
        );
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
        let program = result.program.expect("program");
        match &program.items[0] {
            Item::Function(function) => match &function.body {
                CallableBody::Block(block) => match &block.statements[0] {
                    Stmt::Binding(binding) => {
                        assert_eq!(binding.bindings[0].name, "value");
                    }
                    other => panic!("expected binding, got {other:#?}"),
                },
                other => panic!("expected block body, got {other:#?}"),
            },
            other => panic!("expected function, got {other:#?}"),
        }
    }

    #[test]
    fn parses_lambda_expression() {
        let result = parse("def make() Unit = values.map((x, y) -> x + y)\n");
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    }
}
