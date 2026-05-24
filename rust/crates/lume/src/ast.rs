use crate::source::Span;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Program {
    pub package: Option<PackageDecl>,
    pub imports: Vec<ImportDecl>,
    pub items: Vec<Item>,
    pub span: Option<Span>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PackageDecl {
    pub name: String,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ImportDecl {
    pub path: String,
    pub object_name: Option<String>,
    pub wildcard: bool,
    pub symbols: Vec<ImportSymbol>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ImportSymbol {
    pub name: String,
    pub alias: Option<String>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Annotation {
    pub value: Expr,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Item {
    Function(FunctionDecl),
    Type(TypeDecl),
    Impl(ImplBlock),
    Statement(Stmt),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Visibility {
    Default,
    Public,
    Hidden,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TypeKind {
    Class,
    Record,
    Object,
    Interface,
    Enum,
}

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

#[derive(Debug, Clone, PartialEq)]
pub enum TypeMember {
    Field(FieldDecl),
    Method(MethodDecl),
    Case(EnumCaseDecl),
}

#[derive(Debug, Clone, PartialEq)]
pub struct EnumCaseDecl {
    pub annotations: Vec<Annotation>,
    pub name: String,
    pub fields: Vec<FieldDecl>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ImplBlock {
    pub target: TypeRef,
    pub methods: Vec<MethodDecl>,
    pub span: Span,
}

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

#[derive(Debug, Clone, PartialEq)]
pub enum CallableBody {
    Block(Block),
    Expr(Expr),
}

#[derive(Debug, Clone, PartialEq)]
pub struct FieldDecl {
    pub annotations: Vec<Annotation>,
    pub visibility: Visibility,
    pub mutable: bool,
    pub name: String,
    pub ty: Option<TypeRef>,
    pub initializer: Option<Expr>,
    pub deferred: bool,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TypeParam {
    pub name: String,
    pub bounds: Vec<TypeRef>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Param {
    pub name: String,
    pub ty: Option<TypeRef>,
    pub variadic: bool,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TypeRef {
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

#[derive(Debug, Clone, PartialEq)]
pub struct TupleTypeField {
    pub name: Option<String>,
    pub ty: TypeRef,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RecordTypeField {
    pub name: String,
    pub ty: TypeRef,
    pub span: Span,
}

impl TypeRef {
    pub fn span(&self) -> Span {
        match self {
            TypeRef::Named { span, .. }
            | TypeRef::Tuple { span, .. }
            | TypeRef::Record { span, .. }
            | TypeRef::Function { span, .. } => *span,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Block {
    pub statements: Vec<Stmt>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Stmt {
    Binding(BindingStmt),
    Assignment(AssignmentStmt),
    If(IfStmt),
    Match(MatchStmt),
    While(WhileStmt),
    For(ForStmt),
    Return(ReturnStmt),
    Break(BreakStmt),
    Expr(ExprStmt),
    Unwrap(UnwrapStmt),
    UnwrapBlock(UnwrapBlockStmt),
    LocalFunction(FunctionDecl),
}

impl Stmt {
    pub fn span(&self) -> Span {
        match self {
            Stmt::Binding(stmt) => stmt.span,
            Stmt::Assignment(stmt) => stmt.span,
            Stmt::If(stmt) => stmt.span,
            Stmt::Match(stmt) => stmt.span,
            Stmt::While(stmt) => stmt.span,
            Stmt::For(stmt) => stmt.span,
            Stmt::Return(stmt) => stmt.span,
            Stmt::Break(stmt) => stmt.span,
            Stmt::Expr(stmt) => stmt.span,
            Stmt::Unwrap(stmt) => stmt.span,
            Stmt::UnwrapBlock(stmt) => stmt.span,
            Stmt::LocalFunction(function) => function.span,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct BindingStmt {
    pub visibility: Visibility,
    pub bindings: Vec<Binding>,
    pub values: Vec<Expr>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Binding {
    pub name: String,
    pub ty: Option<TypeRef>,
    pub mutable: bool,
    pub deferred: bool,
    pub span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssignOp {
    Reassign,
    AddAssign,
    SubAssign,
    MulAssign,
    DivAssign,
    ModAssign,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AssignmentStmt {
    pub targets: Vec<Expr>,
    pub operator: AssignOp,
    pub values: Vec<Expr>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct IfStmt {
    pub condition: Option<Expr>,
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
pub struct ForBinding {
    pub bindings: Vec<Binding>,
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
pub struct ExprStmt {
    pub expr: Expr,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct UnwrapStmt {
    pub bindings: Vec<Binding>,
    pub value: Expr,
    pub else_block: Option<Block>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct UnwrapBlockStmt {
    pub clauses: Vec<UnwrapStmt>,
    pub else_block: Option<Block>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Pattern {
    Wildcard {
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
            | Pattern::Binding { span, .. }
            | Pattern::Type { span, .. }
            | Pattern::Literal { span, .. }
            | Pattern::Tuple { span, .. }
            | Pattern::Constructor { span, .. } => *span,
        }
    }
}

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
            | Expr::Unary { span, .. }
            | Expr::Binary { span, .. }
            | Expr::Is { span, .. }
            | Expr::If { span, .. }
            | Expr::Block { span, .. }
            | Expr::Match { span, .. }
            | Expr::ForYield { span, .. }
            | Expr::Lambda { span, .. }
            | Expr::Group { span, .. } => *span,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ElseExprBranch {
    If(Box<Expr>),
    Block(Block),
}

#[derive(Debug, Clone, PartialEq)]
pub struct LambdaParam {
    pub name: String,
    pub ty: Option<TypeRef>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum LambdaBody {
    Expr(Box<Expr>),
    Block(Block),
}

#[derive(Debug, Clone, PartialEq)]
pub struct CallArg {
    pub name: Option<String>,
    pub value: Expr,
    pub span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOp {
    Neg,
    Not,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryOp {
    Colon,
    Or,
    And,
    BitOr,
    BitAnd,
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
    Concat,
    Remove,
    Append,
    Prepend,
    Compose,
}
