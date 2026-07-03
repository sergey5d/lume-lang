//! Parsed Lume syntax tree.
//!
//! This file defines the high-level source-facing representation produced by
//! the Rust parser. The AST keeps the language's rich surface syntax intact so
//! later phases can resolve names, typecheck, and lower into the simpler IR.

use crate::source::Span;

/// A parsed source file with an optional module header, use declarations, and top-level items.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Program {
    pub module: Option<ModuleDecl>,
    pub imports: Vec<ImportDecl>,
    pub items: Vec<Item>,
    pub span: Option<Span>,
}

/// The file-level `module ...` declaration.
#[derive(Debug, Clone, PartialEq)]
pub struct ModuleDecl {
    pub name: String,
    pub span: Span,
}

/// A single `use ...` declaration.
#[derive(Debug, Clone, PartialEq)]
pub struct ImportDecl {
    pub path: String,
    pub single_name: Option<String>,
    pub wildcard: bool,
    pub symbols: Vec<ImportSymbol>,
    pub span: Span,
}

/// One used symbol inside a selective use list.
#[derive(Debug, Clone, PartialEq)]
pub struct ImportSymbol {
    pub name: String,
    pub alias: Option<String>,
    pub span: Span,
}

/// Source-level annotation attached to a declaration.
#[derive(Debug, Clone, PartialEq)]
pub struct Annotation {
    pub value: Expr,
    pub span: Span,
}

/// A top-level AST item.
#[derive(Debug, Clone, PartialEq)]
pub enum Item {
    Function(FunctionDecl),
    Type(TypeDecl),
    Impl(ImplBlock),
    Statement(Stmt),
}

/// Visibility modifier carried by declarations and fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Visibility {
    Default,
    Hidden,
}

/// The surface category of a type declaration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TypeKind {
    Annotation,
    Class,
    Record,
    Single,
    Interface,
    Enum,
}

/// Distinguishes instance-side impls from singleton-side impls.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImplTargetKind {
    Instance,
    Single,
}

/// An `annotation`, `class`, `shape`, `single`, `interface`, or `enum` declaration.
#[derive(Debug, Clone, PartialEq)]
pub struct TypeDecl {
    pub annotations: Vec<Annotation>,
    pub visibility: Visibility,
    pub kind: TypeKind,
    pub name: String,
    pub type_params: Vec<TypeParam>,
    pub with_bounds: Vec<TypeRef>,
    pub members: Vec<TypeMember>,
    pub span: Span,
}

/// A member that can appear inside a type declaration.
#[derive(Debug, Clone, PartialEq)]
pub enum TypeMember {
    Field(FieldDecl),
    Method(MethodDecl),
    Case(EnumCaseDecl),
}

/// A single enum case declaration, including any payload fields.
#[derive(Debug, Clone, PartialEq)]
pub struct EnumCaseDecl {
    pub annotations: Vec<Annotation>,
    pub name: String,
    pub fields: Vec<FieldDecl>,
    pub span: Span,
}

/// An `impl` block that adds methods to an existing type.
#[derive(Debug, Clone, PartialEq)]
pub struct ImplBlock {
    pub target_kind: ImplTargetKind,
    pub target: TypeRef,
    pub methods: Vec<MethodDecl>,
    pub span: Span,
}

/// A top-level function declaration.
#[derive(Debug, Clone, PartialEq)]
pub struct FunctionDecl {
    pub annotations: Vec<Annotation>,
    pub visibility: Visibility,
    pub name: String,
    pub type_params: Vec<TypeParam>,
    pub params: Vec<Param>,
    pub return_type: Option<TypeRef>,
    pub body: CallableBody,
    pub span: Span,
}

/// A method declaration that belongs to a type or anonymous interface literal.
#[derive(Debug, Clone, PartialEq)]
pub struct MethodDecl {
    pub annotations: Vec<Annotation>,
    pub visibility: Visibility,
    pub name: String,
    pub type_params: Vec<TypeParam>,
    pub params: Vec<Param>,
    pub return_type: Option<TypeRef>,
    pub body: Option<CallableBody>,
    pub span: Span,
}

/// The body of a callable, either as a block or a single expression.
#[derive(Debug, Clone, PartialEq)]
pub enum CallableBody {
    Block(Block),
    Expr(Expr),
}

/// A field declared on a type or enum case payload.
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

/// A generic type parameter and its optional bounds.
#[derive(Debug, Clone, PartialEq)]
pub struct TypeParam {
    pub name: String,
    pub bounds: Vec<TypeRef>,
    pub span: Span,
}

/// A named callable parameter.
#[derive(Debug, Clone, PartialEq)]
pub struct Param {
    pub name: String,
    pub ty: Option<TypeRef>,
    pub initializer: Option<Expr>,
    pub variadic: bool,
    pub span: Span,
}

/// A source-level type reference.
#[derive(Debug, Clone, PartialEq)]
pub enum TypeRef {
    Wildcard {
        span: Span,
    },
    Named {
        name: String,
        args: Vec<TypeRef>,
        span: Span,
    },
    Tuple {
        fields: Vec<TupleTypeField>,
        span: Span,
    },
    Record {
        fields: Vec<RecordTypeField>,
        span: Span,
    },
    Function {
        params: Vec<TypeRef>,
        ret: Box<TypeRef>,
        span: Span,
    },
}

/// One positional element inside a tuple type.
#[derive(Debug, Clone, PartialEq)]
pub struct TupleTypeField {
    pub ty: TypeRef,
    pub span: Span,
}

/// One field inside an anonymous shape type.
#[derive(Debug, Clone, PartialEq)]
pub struct RecordTypeField {
    pub name: String,
    pub ty: TypeRef,
    pub span: Span,
}

impl TypeRef {
    pub fn span(&self) -> Span {
        match self {
            TypeRef::Wildcard { span }
            | TypeRef::Named { span, .. }
            | TypeRef::Tuple { span, .. }
            | TypeRef::Record { span, .. }
            | TypeRef::Function { span, .. } => *span,
        }
    }
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

/// A `var`/`def`-style binding statement that introduces one or more names.
#[derive(Debug, Clone, PartialEq)]
pub struct BindingStmt {
    pub visibility: Visibility,
    pub bindings: Vec<Binding>,
    pub values: Vec<Expr>,
    pub destructure: Option<DestructureKind>,
    pub span: Span,
}

/// Which surface syntax introduced a destructuring binding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DestructureKind {
    Tuple,
    Record,
}

/// A single binding introduced by a binding statement or loop form.
#[derive(Debug, Clone, PartialEq)]
pub struct Binding {
    pub name: String,
    pub field_name: Option<String>,
    pub ty: Option<TypeRef>,
    pub mutable: bool,
    pub span: Span,
}

/// One `PATTERN = value` clause inside grouped `let { ... } else ...` or
/// `if let { ... } { ... }` syntax.
#[derive(Debug, Clone, PartialEq)]
pub struct RefutableClause {
    pub pattern: Pattern,
    pub value: Expr,
    pub span: Span,
}

/// Supported assignment operators after parsing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssignOp {
    Assign,
    Reassign,
    AddAssign,
    SubAssign,
    MulAssign,
    DivAssign,
    ModAssign,
}

/// An assignment statement, including compound assignments.
#[derive(Debug, Clone, PartialEq)]
pub struct AssignmentStmt {
    pub targets: Vec<Expr>,
    pub operator: AssignOp,
    pub values: Vec<Expr>,
    pub span: Span,
}

/// A `defer` statement that runs a call or block when the current scope exits.
#[derive(Debug, Clone, PartialEq)]
pub struct DeferStmt {
    pub action: DeferAction,
    pub span: Span,
}

/// The deferred action recorded by a `defer` statement.
#[derive(Debug, Clone, PartialEq)]
pub enum DeferAction {
    Call(Expr),
    Block(Block),
}

/// An `if` statement, including `if let` pattern bindings.
#[derive(Debug, Clone, PartialEq)]
pub enum IfConditionClause {
    Let(RefutableClause),
    Expr(Expr),
}

/// An `if` statement, including `if let` pattern bindings.
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

/// The alternative branch of an `if` statement.
#[derive(Debug, Clone, PartialEq)]
pub enum ElseBranch {
    If(Box<IfStmt>),
    Block(Block),
}

/// A `match` or `partial` statement.
#[derive(Debug, Clone, PartialEq)]
pub struct MatchStmt {
    pub partial: bool,
    pub value: Expr,
    pub cases: Vec<MatchCase>,
    pub span: Span,
}

/// One `case` arm inside a match statement or expression.
#[derive(Debug, Clone, PartialEq)]
pub struct MatchCase {
    pub pattern: Pattern,
    pub guard: Option<Expr>,
    pub body: MatchCaseBody,
    pub span: Span,
}

/// The body attached to a single match arm.
#[derive(Debug, Clone, PartialEq)]
pub enum MatchCaseBody {
    Block(Block),
    Expr(Expr),
}

/// A `while` loop statement.
#[derive(Debug, Clone, PartialEq)]
pub struct WhileStmt {
    pub condition: Expr,
    pub body: Block,
    pub span: Span,
}

/// A `for` loop statement.
#[derive(Debug, Clone, PartialEq)]
pub struct ForStmt {
    pub bindings: Vec<ForBinding>,
    pub body: Block,
    pub span: Span,
}

/// A `let PATTERN = expr else { ... }` statement, or a grouped
/// `let { ... } else { ... }`, whose fallback must exit the current control
/// flow path on match failure, either explicitly or through a `Never`-returning
/// call.
#[derive(Debug, Clone, PartialEq)]
pub struct LetElseStmt {
    pub clauses: Vec<RefutableClause>,
    pub pattern: Pattern,
    pub value: Expr,
    pub else_block: Block,
    pub span: Span,
}

/// Distinguishes the surface keyword used for a pattern binding statement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PatternBindingKind {
    Let,
    Expect,
}

/// A `let PATTERN = expr` / `expect PATTERN = expr` statement, or grouped
/// `let { ... }` / `expect { ... }`.
///
/// `expect` remains assertive and panics on mismatch. Plain `let` is only
/// valid when the pattern is irrefutable.
#[derive(Debug, Clone, PartialEq)]
pub struct PatternBindingStmt {
    pub kind: PatternBindingKind,
    pub clauses: Vec<RefutableClause>,
    pub pattern: Pattern,
    pub value: Expr,
    pub span: Span,
}

/// One generator/binding clause inside a `for` loop or `for ... yield`.
#[derive(Debug, Clone, PartialEq)]
pub struct ForBinding {
    pub bindings: Vec<Binding>,
    pub destructure: Option<DestructureKind>,
    pub pattern: Option<Pattern>,
    pub iterable: Option<Expr>,
    pub values: Vec<Expr>,
    pub span: Span,
}

/// A `return` statement.
#[derive(Debug, Clone, PartialEq)]
pub struct ReturnStmt {
    pub value: Option<Expr>,
    pub span: Span,
}

/// A `break` statement.
#[derive(Debug, Clone, PartialEq)]
pub struct BreakStmt {
    pub span: Span,
}

/// A `continue` statement.
#[derive(Debug, Clone, PartialEq)]
pub struct ContinueStmt {
    pub span: Span,
}

/// An expression used as a statement.
#[derive(Debug, Clone, PartialEq)]
pub struct ExprStmt {
    pub expr: Expr,
    pub span: Span,
}

/// A pattern used by `match`, `partial`, and related destructuring forms.
#[derive(Debug, Clone, PartialEq)]
pub enum Pattern {
    Wildcard {
        span: Span,
    },
    Extract {
        inner: Box<Pattern>,
        span: Span,
    },
    Binding {
        name: String,
        span: Span,
    },
    Type {
        name: Option<String>,
        target: TypeRef,
        span: Span,
    },
    Literal {
        value: Expr,
        span: Span,
    },
    Tuple {
        elements: Vec<Pattern>,
        span: Span,
    },
    Constructor {
        path: Vec<String>,
        args: Vec<Pattern>,
        span: Span,
    },
}

impl Pattern {
    pub fn span(&self) -> Span {
        match self {
            Pattern::Wildcard { span }
            | Pattern::Extract { span, .. }
            | Pattern::Binding { span, .. }
            | Pattern::Type { span, .. }
            | Pattern::Literal { span, .. }
            | Pattern::Tuple { span, .. }
            | Pattern::Constructor { span, .. } => *span,
        }
    }
}

/// A source-level expression.
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
    ListLiteral {
        items: Vec<Expr>,
        span: Span,
    },
    TupleLiteral {
        items: Vec<Expr>,
        span: Span,
    },
    Call {
        callee: Box<Expr>,
        args: Vec<CallArg>,
        uses_brace_syntax: bool,
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
        updates: Vec<CallArg>,
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
    Try {
        value: Box<Expr>,
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
        body: LambdaBody,
        span: Span,
    },
    LiftedChain {
        base: Box<Expr>,
        segments: Vec<LiftedChainSegment>,
        span: Span,
    },
    Group {
        inner: Box<Expr>,
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
            | Expr::ListLiteral { span, .. }
            | Expr::TupleLiteral { span, .. }
            | Expr::Call { span, .. }
            | Expr::Member { span, .. }
            | Expr::Index { span, .. }
            | Expr::RecordUpdate { span, .. }
            | Expr::RecordLiteral { span, .. }
            | Expr::AnonymousInterface { span, .. }
            | Expr::Try { span, .. }
            | Expr::Unary { span, .. }
            | Expr::Binary { span, .. }
            | Expr::Is { span, .. }
            | Expr::TypeOf { span, .. }
            | Expr::If { span, .. }
            | Expr::Block { span, .. }
            | Expr::Match { span, .. }
            | Expr::ForYield { span, .. }
            | Expr::Lambda { span, .. }
            | Expr::LiftedChain { span, .. }
            | Expr::Group { span, .. } => *span,
        }
    }
}

/// The alternative branch of an expression-form `if`.
#[derive(Debug, Clone, PartialEq)]
pub enum ElseExprBranch {
    If(Box<Expr>),
    Block(Block),
}

/// A single lambda parameter.
#[derive(Debug, Clone, PartialEq)]
pub struct LambdaParam {
    pub name: String,
    pub ty: Option<TypeRef>,
    pub destructure: Option<LambdaParamDestructure>,
    pub span: Span,
}

/// A recovered destructured lambda parameter used only for diagnostics.
#[derive(Debug, Clone, PartialEq)]
pub struct LambdaParamDestructure {
    pub kind: DestructureKind,
    pub bindings: Vec<Binding>,
}

/// The body of a lambda expression.
#[derive(Debug, Clone, PartialEq)]
pub enum LambdaBody {
    Expr(Box<Expr>),
    Block(Block),
}

/// One `.->` segment. The body is expressed in terms of `param`.
#[derive(Debug, Clone, PartialEq)]
pub struct LiftedChainSegment {
    pub param: String,
    pub body: Expr,
    pub span: Span,
}

/// A call argument, optionally carrying a source-level name.
#[derive(Debug, Clone, PartialEq)]
pub struct CallArg {
    pub name: Option<String>,
    pub ty: Option<TypeRef>,
    pub value: Expr,
    pub span: Span,
}

/// Supported unary operators in source syntax.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOp {
    Neg,
    Not,
}

/// Supported binary operators in source syntax.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryOp {
    Colon,
    RecordMerge,
    Or,
    And,
    Eq,
    NotEq,
    Less,
    LessEq,
    Greater,
    GreaterEq,
    Add,
    Sub,
    Mul,
    Div,
    Mod,
}
