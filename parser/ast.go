package parser

// Position identifies a source location using 1-based line and column numbers.
type Position struct {
	Line   int `json:"line"`
	Column int `json:"column"`
}

// Span describes the source range covered by a parsed node.
type Span struct {
	Start Position `json:"start"`
	End   Position `json:"end"`
}

// Program is the root parser AST node for a source file.
type Program struct {
	ModuleName string           `json:"moduleName,omitempty"`
	ModuleSpan Span             `json:"moduleSpan,omitempty"`
	Imports    []ImportDecl     `json:"imports,omitempty"`
	Functions  []*FunctionDecl  `json:"functions"`
	Interfaces []*InterfaceDecl `json:"interfaces,omitempty"`
	Classes    []*ClassDecl     `json:"classes,omitempty"`
	Statements []Statement      `json:"statements,omitempty"`
	Span       Span             `json:"span"`
}

// ImportDecl describes a single imported module path.
type ImportDecl struct {
	Path       string         `json:"path"`
	ObjectName string         `json:"objectName,omitempty"`
	Wildcard   bool           `json:"wildcard,omitempty"`
	Symbols    []ImportSymbol `json:"symbols,omitempty"`
	Span       Span           `json:"span"`
}

// ImportSymbol describes a directly imported symbol and its optional alias.
type ImportSymbol struct {
	Name  string `json:"name"`
	Alias string `json:"alias,omitempty"`
	Span  Span   `json:"span"`
}

// Annotation describes metadata attached to a declaration or member.
type Annotation struct {
	Value Expr `json:"value"`
	Span  Span `json:"span"`
}

// TypeRef represents a named, generic, or function type in source.
type TypeRef struct {
	Name           string      `json:"name,omitempty"`
	Arguments      []*TypeRef  `json:"arguments,omitempty"`
	TupleElements  []*TypeRef  `json:"tupleElements,omitempty"`
	TupleNames     []string    `json:"tupleNames,omitempty"`
	RecordFields   []TypeField `json:"recordFields,omitempty"`
	ParameterTypes []*TypeRef  `json:"parameterTypes,omitempty"`
	ReturnType     *TypeRef    `json:"returnType,omitempty"`
	Span           Span        `json:"span"`
}

// TypeField describes a named field in an anonymous record shape.
type TypeField struct {
	Name string   `json:"name"`
	Type *TypeRef `json:"type"`
	Span Span     `json:"span"`
}

// TypeParameter declares a generic type parameter name.
type TypeParameter struct {
	Name   string     `json:"name"`
	Bounds []*TypeRef `json:"bounds,omitempty"`
	Span   Span       `json:"span"`
}

// FunctionDecl describes a top-level function declaration.
type FunctionDecl struct {
	Annotations    []Annotation    `json:"annotations,omitempty"`
	Name           string          `json:"name"`
	TypeParameters []TypeParameter `json:"typeParameters,omitempty"`
	Parameters     []Parameter     `json:"parameters"`
	ReturnType     *TypeRef        `json:"returnType"`
	Body           *BlockStmt      `json:"body"`
	Private        bool            `json:"private,omitempty"`
	Public         bool            `json:"public,omitempty"`
	Span           Span            `json:"span"`
}

// InterfaceDecl describes an interface declaration and its methods.
type InterfaceDecl struct {
	Annotations    []Annotation      `json:"annotations,omitempty"`
	Name           string            `json:"name"`
	Private        bool              `json:"private,omitempty"`
	TypeParameters []TypeParameter   `json:"typeParameters,omitempty"`
	Extends        []*TypeRef        `json:"extends,omitempty"`
	Methods        []InterfaceMethod `json:"methods"`
	Span           Span              `json:"span"`
}

// InterfaceMethod describes a method signature inside an interface.
type InterfaceMethod struct {
	Annotations    []Annotation    `json:"annotations,omitempty"`
	Name           string          `json:"name"`
	TypeParameters []TypeParameter `json:"typeParameters,omitempty"`
	Parameters     []Parameter     `json:"parameters"`
	ReturnType     *TypeRef        `json:"returnType"`
	Body           *BlockStmt      `json:"body,omitempty"`
	Span           Span            `json:"span"`
}

// ClassDecl describes a class declaration, its fields, and its methods.
type ClassDecl struct {
	Annotations    []Annotation    `json:"annotations,omitempty"`
	Name           string          `json:"name"`
	Private        bool            `json:"private,omitempty"`
	Object         bool            `json:"object,omitempty"`
	Record         bool            `json:"record,omitempty"`
	Enum           bool            `json:"enum,omitempty"`
	TypeParameters []TypeParameter `json:"typeParameters,omitempty"`
	Implements     []*TypeRef      `json:"implements,omitempty"`
	Fields         []FieldDecl     `json:"fields,omitempty"`
	Methods        []*MethodDecl   `json:"methods,omitempty"`
	Cases          []EnumCaseDecl  `json:"cases,omitempty"`
	Span           Span            `json:"span"`
}

// EnumCaseDecl describes a single enum case, including shared-field assignments and payload fields.
type EnumCaseDecl struct {
	Annotations []Annotation         `json:"annotations,omitempty"`
	Name        string               `json:"name"`
	Fields      []FieldDecl          `json:"fields,omitempty"`
	Assignments []EnumCaseAssignment `json:"assignments,omitempty"`
	Methods     []*MethodDecl        `json:"methods,omitempty"`
	Span        Span                 `json:"span"`
}

// EnumCaseAssignment assigns a shared enum field for a given case.
type EnumCaseAssignment struct {
	Name  string `json:"name"`
	Value Expr   `json:"value"`
	Span  Span   `json:"span"`
}

// FieldDecl describes a class field declaration.
type FieldDecl struct {
	Annotations []Annotation `json:"annotations,omitempty"`
	Name        string       `json:"name"`
	Type        *TypeRef     `json:"type"`
	Initializer Expr         `json:"initializer,omitempty"`
	Mutable     bool         `json:"mutable,omitempty"`
	Deferred    bool         `json:"deferred,omitempty"`
	Private     bool         `json:"private,omitempty"`
	Span        Span         `json:"span"`
}

// MethodDecl describes a class method or constructor declaration.
type MethodDecl struct {
	Annotations    []Annotation    `json:"annotations,omitempty"`
	Name           string          `json:"name"`
	TypeParameters []TypeParameter `json:"typeParameters,omitempty"`
	Parameters     []Parameter     `json:"parameters"`
	ReturnType     *TypeRef        `json:"returnType,omitempty"`
	Body           *BlockStmt      `json:"body"`
	Partial        bool            `json:"partial,omitempty"`
	Operator       bool            `json:"operator,omitempty"`
	Private        bool            `json:"private,omitempty"`
	Constructor    bool            `json:"constructor,omitempty"`
	Span           Span            `json:"span"`
}

// Parameter describes a named typed parameter in a callable signature.
type Parameter struct {
	Name     string   `json:"name"`
	Type     *TypeRef `json:"type"`
	Variadic bool     `json:"variadic,omitempty"`
	Span     Span     `json:"span"`
}

// CallArg represents a positional or named argument in a call expression.
type CallArg struct {
	Name  string `json:"name,omitempty"`
	Value Expr   `json:"value"`
	Span  Span   `json:"span"`
}

// BlockStmt is a braced sequence of statements.
type BlockStmt struct {
	Statements []Statement `json:"statements"`
	Span       Span        `json:"span"`
}

// Statement is implemented by all parser AST statement nodes.
type Statement interface {
	statementNode()
}

// Pattern is implemented by all parser AST match-pattern nodes.
type Pattern interface {
	patternNode()
}

// LocalFunctionStmt declares a named local function inside a block.
type LocalFunctionStmt struct {
	Function *FunctionDecl `json:"function"`
	Span     Span          `json:"span"`
}

func (*LocalFunctionStmt) statementNode() {}

// Expr is implemented by all parser AST expression nodes.
type Expr interface {
	exprNode()
}

// ValStmt declares one or more bindings with matching initializer expressions.
type ValStmt struct {
	Bindings []Binding `json:"bindings"`
	Values   []Expr    `json:"values"`
	Public   bool      `json:"public,omitempty"`
	Span     Span      `json:"span"`
}

// Binding describes a single declared name in a binding statement.
type Binding struct {
	Name     string   `json:"name"`
	Type     *TypeRef `json:"type,omitempty"`
	Mutable  bool     `json:"mutable,omitempty"`
	Deferred bool     `json:"deferred,omitempty"`
	Span     Span     `json:"span"`
}

func (*ValStmt) statementNode() {}

// UnwrapStmt extracts a success value from an unwrappable value or returns early on failure.
type UnwrapStmt struct {
	Bindings []Binding `json:"bindings"`
	Value    Expr      `json:"value"`
	Span     Span      `json:"span"`
}

func (*UnwrapStmt) statementNode() {}

// UnwrapBlockStmt evaluates a sequence of unwrap bindings and returns early on the first failure.
type UnwrapBlockStmt struct {
	Clauses []*UnwrapStmt `json:"clauses"`
	Span    Span          `json:"span"`
}

func (*UnwrapBlockStmt) statementNode() {}

// GuardStmt extracts a success value from an unwrappable value or evaluates a fallback block on failure.
type GuardStmt struct {
	Bindings []Binding  `json:"bindings"`
	Value    Expr       `json:"value"`
	Fallback *BlockStmt `json:"fallback"`
	Span     Span       `json:"span"`
}

func (*GuardStmt) statementNode() {}

// GuardBlockStmt evaluates a sequence of unwrap bindings and runs a fallback block if any binding fails.
type GuardBlockStmt struct {
	Clauses  []*UnwrapStmt `json:"clauses"`
	Fallback *BlockStmt    `json:"fallback"`
	Span     Span          `json:"span"`
}

func (*GuardBlockStmt) statementNode() {}

// RefutableClause is one refutable `PATTERN = value` clause inside a grouped
// `let { ... } else ...` or `if let { ... } { ... }` form.
type RefutableClause struct {
	Pattern Pattern `json:"pattern"`
	Value   Expr    `json:"value"`
	Span    Span    `json:"span"`
}

// LetElseStmt matches one or more values against refutable patterns and
// implicitly returns from the current callable on failure.
type LetElseStmt struct {
	Pattern  Pattern           `json:"pattern,omitempty"`
	Value    Expr              `json:"value,omitempty"`
	Clauses  []RefutableClause `json:"clauses,omitempty"`
	Fallback *BlockStmt        `json:"fallback"`
	Span     Span              `json:"span"`
}

func (*LetElseStmt) statementNode() {}

// AssignmentStmt writes a value to an existing assignment target.
type AssignmentStmt struct {
	Target   Expr   `json:"target"`
	Operator string `json:"operator"`
	Value    Expr   `json:"value"`
	Span     Span   `json:"span"`
}

func (*AssignmentStmt) statementNode() {}

// MultiAssignmentStmt writes multiple values to multiple existing assignment targets.
type MultiAssignmentStmt struct {
	Targets  []Expr `json:"targets"`
	Operator string `json:"operator"`
	Values   []Expr `json:"values"`
	Span     Span   `json:"span"`
}

func (*MultiAssignmentStmt) statementNode() {}

// IfStmt represents an if / else-if / else chain, including `if let`
// pattern bindings.
type IfStmt struct {
	Condition      Expr              `json:"condition,omitempty"`
	Pattern        Pattern           `json:"pattern,omitempty"`
	PatternValue   Expr              `json:"patternValue,omitempty"`
	PatternClauses []RefutableClause `json:"patternClauses,omitempty"`
	Bindings       []Binding         `json:"bindings,omitempty"`
	BindingValue   Expr              `json:"bindingValue,omitempty"`
	Then           *BlockStmt        `json:"then"`
	ElseIf         *IfStmt           `json:"elseIf,omitempty"`
	Else           *BlockStmt        `json:"else,omitempty"`
	Span           Span              `json:"span"`
}

func (*IfStmt) statementNode() {}

// MatchStmt represents a match statement with pattern cases.
type MatchStmt struct {
	Partial bool        `json:"partial,omitempty"`
	Value   Expr        `json:"value"`
	Cases   []MatchCase `json:"cases"`
	Span    Span        `json:"span"`
}

func (*MatchStmt) statementNode() {}

// MatchCase represents a single pattern/body case in a match statement.
type MatchCase struct {
	Pattern Pattern    `json:"pattern"`
	Guard   Expr       `json:"guard,omitempty"`
	Body    *BlockStmt `json:"body"`
	Expr    Expr       `json:"expr,omitempty"`
	Span    Span       `json:"span"`
}

// WhileStmt represents a conditional loop.
type WhileStmt struct {
	Condition Expr       `json:"condition"`
	Body      *BlockStmt `json:"body"`
	Span      Span       `json:"span"`
}

func (*WhileStmt) statementNode() {}

// ForStmt represents foreach and yield-style loops.
type ForStmt struct {
	Bindings  []ForBinding `json:"bindings,omitempty"`
	Body      *BlockStmt   `json:"body,omitempty"`
	YieldBody *BlockStmt   `json:"yieldBody,omitempty"`
	Span      Span         `json:"span"`
}

func (*ForStmt) statementNode() {}

// ForBinding represents either a generator clause or an immutable local binding
// inside a for/yield binding list.
type ForBinding struct {
	Bindings []Binding `json:"bindings,omitempty"`
	Iterable Expr      `json:"iterable,omitempty"`
	Values   []Expr    `json:"values,omitempty"`
	Span     Span      `json:"span"`
}

// ReturnStmt returns a value from a callable body.
type ReturnStmt struct {
	Value Expr `json:"value"`
	Span  Span `json:"span"`
}

func (*ReturnStmt) statementNode() {}

// BreakStmt exits the nearest loop.
type BreakStmt struct {
	Span Span `json:"span"`
}

func (*BreakStmt) statementNode() {}

// ExprStmt wraps an expression used as a statement.
type ExprStmt struct {
	Expr Expr `json:"expr"`
	Span Span `json:"span"`
}

func (*ExprStmt) statementNode() {}

// WildcardPattern matches any value and binds nothing.
type WildcardPattern struct {
	Span Span `json:"span"`
}

func (*WildcardPattern) patternNode() {}

// BindingPattern binds the matched value to a local name.
type BindingPattern struct {
	Name string `json:"name"`
	Span Span   `json:"span"`
}

func (*BindingPattern) patternNode() {}

// TypePattern matches by runtime type and optionally binds the matched value.
type TypePattern struct {
	Name   string   `json:"name,omitempty"`
	Target *TypeRef `json:"target"`
	Span   Span     `json:"span"`
}

func (*TypePattern) patternNode() {}

// LiteralPattern matches a literal value exactly.
type LiteralPattern struct {
	Value Expr `json:"value"`
	Span  Span `json:"span"`
}

func (*LiteralPattern) patternNode() {}

// TuplePattern matches a destructurable value by position.
type TuplePattern struct {
	Elements []Pattern `json:"elements"`
	Span     Span      `json:"span"`
}

func (*TuplePattern) patternNode() {}

// ConstructorPattern matches an enum case and its payload fields.
type ConstructorPattern struct {
	Path []string  `json:"path"`
	Args []Pattern `json:"args,omitempty"`
	Span Span      `json:"span"`
}

func (*ConstructorPattern) patternNode() {}

// Identifier is a name reference expression.
type Identifier struct {
	Name string `json:"name"`
	Span Span   `json:"span"`
}

func (*Identifier) exprNode() {}

// PlaceholderExpr represents the `_` placeholder expression.
type PlaceholderExpr struct {
	Span Span `json:"span"`
}

func (*PlaceholderExpr) exprNode() {}

// IntegerLiteral is a source integer literal.
type IntegerLiteral struct {
	Value string `json:"value"`
	Span  Span   `json:"span"`
}

func (*IntegerLiteral) exprNode() {}

// FloatLiteral is a source floating-point literal.
type FloatLiteral struct {
	Value string `json:"value"`
	Span  Span   `json:"span"`
}

func (*FloatLiteral) exprNode() {}

// RuneLiteral is a source rune literal.
type RuneLiteral struct {
	Value string `json:"value"`
	Span  Span   `json:"span"`
}

func (*RuneLiteral) exprNode() {}

// BoolLiteral is a source boolean literal.
type BoolLiteral struct {
	Value bool `json:"value"`
	Span  Span `json:"span"`
}

func (*BoolLiteral) exprNode() {}

// StringLiteral is a source string literal.
type StringLiteral struct {
	Value string `json:"value"`
	Span  Span   `json:"span"`
}

func (*StringLiteral) exprNode() {}

// UnitLiteral is the unit value expression written as ().
type UnitLiteral struct {
	Span Span `json:"span"`
}

func (*UnitLiteral) exprNode() {}

// ListLiteral is a bracketed list literal expression.
type ListLiteral struct {
	Elements []Expr `json:"elements"`
	Span     Span   `json:"span"`
}

func (*ListLiteral) exprNode() {}

// TupleLiteral is a parenthesized tuple literal expression.
type TupleLiteral struct {
	Elements []Expr `json:"elements"`
	Span     Span   `json:"span"`
}

func (*TupleLiteral) exprNode() {}

// CallExpr applies a callee expression to positional and named arguments.
type CallExpr struct {
	Callee Expr      `json:"callee"`
	Args   []CallArg `json:"args"`
	Span   Span      `json:"span"`
}

func (*CallExpr) exprNode() {}

// MemberExpr reads a named member from a receiver expression.
type MemberExpr struct {
	Receiver Expr   `json:"receiver"`
	Name     string `json:"name"`
	Span     Span   `json:"span"`
}

func (*MemberExpr) exprNode() {}

// IndexExpr indexes into a receiver expression with another expression.
type IndexExpr struct {
	Receiver Expr `json:"receiver"`
	Index    Expr `json:"index"`
	Span     Span `json:"span"`
}

func (*IndexExpr) exprNode() {}

// RecordUpdateExpr copies a record value and overrides selected fields.
type RecordUpdateExpr struct {
	Receiver Expr      `json:"receiver"`
	Updates  []CallArg `json:"updates"`
	Span     Span      `json:"span"`
}

func (*RecordUpdateExpr) exprNode() {}

// AnonymousRecordExpr creates a structural anonymous record value.
type AnonymousRecordExpr struct {
	Fields []CallArg `json:"fields"`
	Values []Expr    `json:"values,omitempty"`
	Span   Span      `json:"span"`
}

func (*AnonymousRecordExpr) exprNode() {}

// AnonymousInterfaceExpr creates an anonymous object implementing one or more interfaces.
type AnonymousInterfaceExpr struct {
	Interfaces []*TypeRef    `json:"interfaces"`
	Methods    []*MethodDecl `json:"methods"`
	Span       Span          `json:"span"`
}

func (*AnonymousInterfaceExpr) exprNode() {}

// IfExpr is an if / else expression whose value comes from the chosen branch.
type IfExpr struct {
	Condition Expr       `json:"condition"`
	Then      *BlockStmt `json:"then"`
	Else      *BlockStmt `json:"else"`
	Span      Span       `json:"span"`
}

func (*IfExpr) exprNode() {}

// BlockExpr evaluates a braced block and yields the value of its final statement.
type BlockExpr struct {
	Body *BlockStmt `json:"body"`
	Span Span       `json:"span"`
}

func (*BlockExpr) exprNode() {}

// MatchExpr is a match expression whose value comes from the matched case.
type MatchExpr struct {
	Partial bool        `json:"partial,omitempty"`
	Value   Expr        `json:"value"`
	Cases   []MatchCase `json:"cases"`
	Span    Span        `json:"span"`
}

func (*MatchExpr) exprNode() {}

// ForYieldExpr is a yield-style for expression that evaluates to a list.
type ForYieldExpr struct {
	Bindings  []ForBinding `json:"bindings"`
	YieldBody *BlockStmt   `json:"yieldBody"`
	Span      Span         `json:"span"`
}

func (*ForYieldExpr) exprNode() {}

// LambdaParameter describes a single lambda parameter.
type LambdaParameter struct {
	Name string   `json:"name"`
	Type *TypeRef `json:"type,omitempty"`
	Span Span     `json:"span"`
}

// LambdaExpr represents an expression-bodied or block-bodied lambda.
type LambdaExpr struct {
	Parameters []LambdaParameter `json:"parameters"`
	Body       Expr              `json:"body,omitempty"`
	BlockBody  *BlockStmt        `json:"blockBody,omitempty"`
	Span       Span              `json:"span"`
}

func (*LambdaExpr) exprNode() {}

// BinaryExpr applies a binary operator to two expressions.
type BinaryExpr struct {
	Left     Expr   `json:"left"`
	Operator string `json:"operator"`
	Right    Expr   `json:"right"`
	Span     Span   `json:"span"`
}

func (*BinaryExpr) exprNode() {}

// IsExpr checks whether a value conforms to a type at runtime.
type IsExpr struct {
	Left   Expr     `json:"left"`
	Target *TypeRef `json:"target"`
	Span   Span     `json:"span"`
}

func (*IsExpr) exprNode() {}

// UnaryExpr applies a unary operator to one expression.
type UnaryExpr struct {
	Operator string `json:"operator"`
	Right    Expr   `json:"right"`
	Span     Span   `json:"span"`
}

func (*UnaryExpr) exprNode() {}

// GroupExpr preserves parenthesized expression structure.
type GroupExpr struct {
	Inner Expr `json:"inner"`
	Span  Span `json:"span"`
}

func (*GroupExpr) exprNode() {}

// TryExpr unwraps a success value or implicitly returns the failure from the current callable.
type TryExpr struct {
	Value Expr `json:"value"`
	Span  Span `json:"span"`
}

func (*TryExpr) exprNode() {}
