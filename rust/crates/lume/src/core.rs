//! Expression- and body-level core language.
//!
//! This is the first desugaring slice between the source-facing AST and the
//! execution-oriented IR. For now it focuses on callable bodies, blocks,
//! statements, and expressions. It intentionally keeps most source structure,
//! but already removes a few surface-only details:
//! - grouped expressions are eliminated
//! - call syntax uses a semantic `CallStyle` instead of parser-specific flags
//! - lambda bodies are normalized to a single expression form

use crate::{ast, source::Span};

pub type Annotation = ast::Annotation;
pub type Visibility = ast::Visibility;
pub type TypeParam = ast::TypeParam;
pub type Param = ast::Param;
pub type TypeRef = ast::TypeRef;
pub type AssignOp = ast::AssignOp;
pub type DestructureKind = ast::DestructureKind;
pub type Binding = ast::Binding;
pub type Pattern = ast::Pattern;
pub type BinaryOp = ast::BinaryOp;
pub type UnaryOp = ast::UnaryOp;
pub type LambdaParam = ast::LambdaParam;

/// Distinguishes normal `(...)` calls from trailing brace calls.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallStyle {
    Paren,
    Brace,
}

/// A top-level or local function body after body-level desugaring.
#[derive(Debug, Clone, PartialEq)]
pub struct FunctionDecl {
    pub annotations: Vec<Annotation>,
    pub visibility: Visibility,
    pub name: String,
    pub type_params: Vec<TypeParam>,
    pub type_conditions: Vec<ast::GenericCondition>,
    pub params: Vec<Param>,
    pub return_type: Option<TypeRef>,
    pub body: CallableBody,
    pub span: Span,
}

/// A method body after body-level desugaring.
#[derive(Debug, Clone, PartialEq)]
pub struct MethodDecl {
    pub annotations: Vec<Annotation>,
    pub visibility: Visibility,
    pub name: String,
    pub type_params: Vec<TypeParam>,
    pub type_conditions: Vec<ast::GenericCondition>,
    pub params: Vec<Param>,
    pub return_type: Option<TypeRef>,
    pub body: Option<CallableBody>,
    pub span: Span,
}

/// A field in an anonymous object after its initializer has been desugared.
#[derive(Debug, Clone, PartialEq)]
pub struct FieldDecl {
    pub annotations: Vec<Annotation>,
    pub visibility: Visibility,
    pub mutable: bool,
    pub name: String,
    pub ty: Option<TypeRef>,
    pub initializer: Option<Expr>,
    pub span: Span,
}

/// The body of a callable, either as a block or a single expression.
#[derive(Debug, Clone, PartialEq)]
pub enum CallableBody {
    Block(Block),
    Expr(Expr),
}

/// A block of statements delimited by braces.
#[derive(Debug, Clone, PartialEq)]
pub struct Block {
    pub statements: Vec<Stmt>,
    pub span: Span,
}

/// A statement form that can appear inside a block.
#[derive(Debug, Clone, PartialEq)]
pub enum Stmt {
    Binding(BindingStmt),
    PatternBinding(PatternBindingStmt),
    Assignment(AssignmentStmt),
    Defer(DeferStmt),
    If(IfStmt),
    Match(MatchStmt),
    While(WhileStmt),
    For(ForStmt),
    LetElse(LetElseStmt),
    Return(ReturnStmt),
    Break(BreakStmt),
    Continue(ContinueStmt),
    Expr(ExprStmt),
    LocalFunction(FunctionDecl),
}

impl Stmt {
    pub fn span(&self) -> Span {
        match self {
            Stmt::Binding(stmt) => stmt.span,
            Stmt::PatternBinding(stmt) => stmt.span,
            Stmt::Assignment(stmt) => stmt.span,
            Stmt::Defer(stmt) => stmt.span,
            Stmt::If(stmt) => stmt.span,
            Stmt::Match(stmt) => stmt.span,
            Stmt::While(stmt) => stmt.span,
            Stmt::For(stmt) => stmt.span,
            Stmt::LetElse(stmt) => stmt.span,
            Stmt::Return(stmt) => stmt.span,
            Stmt::Break(stmt) => stmt.span,
            Stmt::Continue(stmt) => stmt.span,
            Stmt::Expr(stmt) => stmt.span,
            Stmt::LocalFunction(function) => function.span,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct BindingStmt {
    pub visibility: Visibility,
    pub bindings: Vec<Binding>,
    pub values: Vec<Expr>,
    pub destructure: Option<DestructureKind>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RefutableClause {
    pub pattern: Pattern,
    pub value: Expr,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AssignmentStmt {
    pub targets: Vec<Expr>,
    pub operator: AssignOp,
    pub values: Vec<Expr>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DeferStmt {
    pub action: DeferAction,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum DeferAction {
    Call(Expr),
    Block(Block),
}

#[derive(Debug, Clone, PartialEq)]
pub enum IfConditionClause {
    Let(RefutableClause),
    Expr(Expr),
}

#[derive(Debug, Clone, PartialEq)]
pub struct IfStmt {
    pub condition: Option<Expr>,
    pub condition_clauses: Vec<IfConditionClause>,
    pub pattern: Option<Pattern>,
    pub pattern_value: Option<Expr>,
    pub pattern_clauses: Vec<RefutableClause>,
    pub bindings: Vec<Binding>,
    pub binding_value: Option<Expr>,
    pub then_block: Block,
    pub else_branch: Option<ElseBranch>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ElseBranch {
    If(Box<IfStmt>),
    Block(Block),
}

#[derive(Debug, Clone, PartialEq)]
pub struct MatchStmt {
    pub partial: bool,
    pub value: Expr,
    pub cases: Vec<MatchCase>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MatchCase {
    pub pattern: Pattern,
    pub guard: Option<Expr>,
    pub body: MatchCaseBody,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum MatchCaseBody {
    Block(Block),
    Expr(Expr),
}

#[derive(Debug, Clone, PartialEq)]
pub struct WhileStmt {
    pub condition: Expr,
    pub body: Block,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ForStmt {
    pub bindings: Vec<ForBinding>,
    pub body: Block,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LetElseStmt {
    pub clauses: Vec<RefutableClause>,
    pub pattern: Pattern,
    pub value: Expr,
    pub else_block: Block,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PatternBindingStmt {
    pub clauses: Vec<RefutableClause>,
    pub pattern: Pattern,
    pub value: Expr,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ForBinding {
    pub bindings: Vec<Binding>,
    pub destructure: Option<DestructureKind>,
    pub pattern: Option<Pattern>,
    pub iterable: Option<Expr>,
    pub values: Vec<Expr>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ReturnStmt {
    pub value: Option<Expr>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BreakStmt {
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ContinueStmt {
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ExprStmt {
    pub expr: Expr,
    pub span: Span,
}

/// A core expression used inside callable bodies.
#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Identifier {
        name: String,
        span: Span,
    },
    Placeholder {
        span: Span,
    },
    Integer {
        raw: String,
        span: Span,
    },
    Float {
        raw: String,
        span: Span,
    },
    String {
        raw: String,
        span: Span,
    },
    Bool {
        value: bool,
        span: Span,
    },
    Unit {
        span: Span,
    },
    Spread {
        value: Box<Expr>,
        span: Span,
    },
    ListLiteral {
        items: Vec<Expr>,
        span: Span,
    },
    TupleLiteral {
        items: Vec<Expr>,
        span: Span,
    },
    ShapeLiteral {
        items: Vec<Expr>,
        span: Span,
    },
    Call {
        callee: Box<Expr>,
        args: Vec<CallArg>,
        style: CallStyle,
        span: Span,
    },
    Member {
        receiver: Box<Expr>,
        name: String,
        span: Span,
    },
    Index {
        receiver: Box<Expr>,
        index: Box<Expr>,
        span: Span,
    },
    RecordUpdate {
        receiver: Box<Expr>,
        patch: Box<Expr>,
        span: Span,
    },
    RecordLiteral {
        fields: Vec<CallArg>,
        values: Vec<Expr>,
        span: Span,
    },
    AnonymousInterface {
        interfaces: Vec<TypeRef>,
        methods: Vec<MethodDecl>,
        span: Span,
    },
    AnonymousObject {
        fields: Vec<FieldDecl>,
        methods: Vec<MethodDecl>,
        span: Span,
    },
    Try {
        value: Box<Expr>,
        span: Span,
    },
    ExtractOr {
        value: Box<Expr>,
        fallback: Box<Expr>,
        span: Span,
    },
    Return {
        value: Option<Box<Expr>>,
        span: Span,
    },
    Break {
        span: Span,
    },
    Continue {
        span: Span,
    },
    Unary {
        op: UnaryOp,
        expr: Box<Expr>,
        span: Span,
    },
    Binary {
        left: Box<Expr>,
        op: BinaryOp,
        right: Box<Expr>,
        span: Span,
    },
    Is {
        left: Box<Expr>,
        target: TypeRef,
        span: Span,
    },
    TypeOf {
        ty: TypeRef,
        span: Span,
    },
    If {
        condition: Box<Expr>,
        then_block: Block,
        else_branch: Box<ElseExprBranch>,
        span: Span,
    },
    Block {
        body: Block,
        span: Span,
    },
    Match {
        partial: bool,
        value: Box<Expr>,
        cases: Vec<MatchCase>,
        span: Span,
    },
    ForYield {
        bindings: Vec<ForBinding>,
        yield_body: Block,
        span: Span,
    },
    Lambda {
        params: Vec<LambdaParam>,
        body: Box<Expr>,
        span: Span,
    },
}

impl Expr {
    pub fn span(&self) -> Span {
        match self {
            Expr::Identifier { span, .. }
            | Expr::Placeholder { span }
            | Expr::Integer { span, .. }
            | Expr::Float { span, .. }
            | Expr::String { span, .. }
            | Expr::Bool { span, .. }
            | Expr::Unit { span }
            | Expr::Spread { span, .. }
            | Expr::ListLiteral { span, .. }
            | Expr::TupleLiteral { span, .. }
            | Expr::ShapeLiteral { span, .. }
            | Expr::Call { span, .. }
            | Expr::Member { span, .. }
            | Expr::Index { span, .. }
            | Expr::RecordUpdate { span, .. }
            | Expr::RecordLiteral { span, .. }
            | Expr::AnonymousInterface { span, .. }
            | Expr::AnonymousObject { span, .. }
            | Expr::Try { span, .. }
            | Expr::ExtractOr { span, .. }
            | Expr::Return { span, .. }
            | Expr::Break { span }
            | Expr::Continue { span }
            | Expr::Unary { span, .. }
            | Expr::Binary { span, .. }
            | Expr::Is { span, .. }
            | Expr::TypeOf { span, .. }
            | Expr::If { span, .. }
            | Expr::Block { span, .. }
            | Expr::Match { span, .. }
            | Expr::ForYield { span, .. }
            | Expr::Lambda { span, .. } => *span,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ElseExprBranch {
    If(Box<Expr>),
    Block(Block),
}

#[derive(Debug, Clone, PartialEq)]
pub struct CallArg {
    pub name: Option<String>,
    pub ty: Option<TypeRef>,
    pub value: Expr,
    pub span: Span,
}
