package typed

import (
	"a-lang/parser"
	"a-lang/typecheck"
)

// BindingMode records whether a typed binding is immutable or mutable.
type BindingMode string

const (
	BindingImmutable BindingMode = "immutable"
	BindingMutable   BindingMode = "mutable"
)

// InitMode records whether a binding/field is initialized immediately or later.
type InitMode string

const (
	InitImmediate InitMode = "immediate"
	InitDeferred  InitMode = "deferred"
)

// SymbolKind classifies the semantic identity attached to a typed node.
type SymbolKind string

const (
	SymbolBinding   SymbolKind = "binding"
	SymbolParameter SymbolKind = "parameter"
	SymbolField     SymbolKind = "field"
	SymbolMethod    SymbolKind = "method"
	SymbolFunction  SymbolKind = "function"
	SymbolClass     SymbolKind = "class"
	SymbolInterface SymbolKind = "interface"
	SymbolThis      SymbolKind = "this"
)

// CallDispatch records how a resolved call should dispatch at runtime.
type CallDispatch string

const (
	DispatchStatic    CallDispatch = "static"
	DispatchVirtual   CallDispatch = "virtual"
	DispatchConstruct CallDispatch = "construct"
)

// SymbolRef is a durable semantic reference for a resolved declaration.
type SymbolRef struct {
	ID    int
	Kind  SymbolKind
	Name  string
	Owner string
	Span  parser.Span
}

// Program is the typed semantic root built from a parser program.
type Program struct {
	Functions  []*FunctionDecl
	Interfaces []*InterfaceDecl
	Classes    []*ClassDecl
	Globals    []Stmt
	Span       parser.Span
}

// TypeParameter carries a generic type parameter name through the typed tree.
type TypeParameter struct {
	Name   string
	Bounds []*typecheck.Type
	Span   parser.Span
}

// Parameter is a typed callable parameter with an attached symbol.
type Parameter struct {
	Name   string
	Type   *typecheck.Type
	Symbol SymbolRef
	Span   parser.Span
}

// FunctionDecl is a typed top-level function declaration.
type FunctionDecl struct {
	Name           string
	TypeParameters []TypeParameter
	Parameters     []Parameter
	ReturnType     *typecheck.Type
	Body           *BlockStmt
	Symbol         SymbolRef
	Span           parser.Span
}

// InterfaceMethod is a typed interface method signature.
type InterfaceMethod struct {
	Name           string
	TypeParameters []TypeParameter
	Parameters     []Parameter
	ReturnType     *typecheck.Type
	Span           parser.Span
}

// InterfaceDecl is a typed interface declaration.
type InterfaceDecl struct {
	Name           string
	TypeParameters []TypeParameter
	Extends        []*typecheck.Type
	Methods        []InterfaceMethod
	Symbol         SymbolRef
	Span           parser.Span
}

// FieldDecl is a typed class field declaration.
type FieldDecl struct {
	Name     string
	Type     *typecheck.Type
	Mode     BindingMode
	InitMode InitMode
	Init     Expr
	Private  bool
	Symbol   SymbolRef
	Span     parser.Span
}

// MethodDecl is a typed class method or constructor declaration.
type MethodDecl struct {
	Name           string
	TypeParameters []TypeParameter
	Parameters     []Parameter
	ReturnType     *typecheck.Type
	Body           *BlockStmt
	Operator       bool
	Private        bool
	Constructor    bool
	Symbol         SymbolRef
	Span           parser.Span
}

// ClassDecl is a typed class declaration including resolved interface types.
type ClassDecl struct {
	Name           string
	Object         bool
	Record         bool
	Enum           bool
	TypeParameters []TypeParameter
	Interfaces     []*typecheck.Type
	Fields         []FieldDecl
	Methods        []*MethodDecl
	Cases          []parser.EnumCaseDecl
	Symbol         SymbolRef
	Span           parser.Span
}

// Stmt is implemented by all typed statement nodes.
type Stmt interface {
	stmtNode()
	GetSpan() parser.Span
}

// Expr is implemented by all typed expression nodes.
type Expr interface {
	exprNode()
	GetSpan() parser.Span
	GetType() *typecheck.Type
}

// BlockStmt is a typed block of statements.
type BlockStmt struct {
	Statements []Stmt
	Span       parser.Span
}

// BindingDecl is a typed declaration of a single named binding.
type BindingDecl struct {
	Name     string
	Type     *typecheck.Type
	Mode     BindingMode
	InitMode InitMode
	Init     Expr
	Symbol   SymbolRef
	Span     parser.Span
}

// BindingStmt groups one or more typed binding declarations.
type BindingStmt struct {
	Bindings []BindingDecl
	Span     parser.Span
}

func (*BindingStmt) stmtNode()              {}
func (s *BindingStmt) GetSpan() parser.Span { return s.Span }

// AssignmentStmt is a typed write to an assignment target.
type AssignmentStmt struct {
	Target   Expr
	Operator string
	Value    Expr
	Span     parser.Span
}

func (*AssignmentStmt) stmtNode()              {}
func (s *AssignmentStmt) GetSpan() parser.Span { return s.Span }

// MultiAssignmentStmt is a typed write to multiple assignment targets.
type MultiAssignmentStmt struct {
	Targets  []Expr
	Operator string
	Values   []Expr
	Span     parser.Span
}

func (*MultiAssignmentStmt) stmtNode()              {}
func (s *MultiAssignmentStmt) GetSpan() parser.Span { return s.Span }

// IfStmt is a typed if / else-if / else chain.
type IfStmt struct {
	Condition    Expr
	Bindings     []BindingDecl
	BindingValue Expr
	Then         *BlockStmt
	ElseIf       *IfStmt
	Else         *BlockStmt
	Span         parser.Span
}

func (*IfStmt) stmtNode()              {}
func (s *IfStmt) GetSpan() parser.Span { return s.Span }

// WhileStmt is a typed conditional loop.
type WhileStmt struct {
	Condition Expr
	Body      *BlockStmt
	Span      parser.Span
}

func (*WhileStmt) stmtNode()              {}
func (s *WhileStmt) GetSpan() parser.Span { return s.Span }

// ForBinding is a typed loop binding with an inferred element type.
type ForBinding struct {
	Bindings []BindingDecl
	Iterable Expr
	Values   []Expr
	Span     parser.Span
}

// ForStmt is a typed loop including optional yield-body form.
type ForStmt struct {
	Bindings  []ForBinding
	Body      *BlockStmt
	YieldBody *BlockStmt
	Span      parser.Span
}

func (*ForStmt) stmtNode()              {}
func (s *ForStmt) GetSpan() parser.Span { return s.Span }

// ReturnStmt is a typed return statement.
type ReturnStmt struct {
	Value Expr
	Span  parser.Span
}

func (*ReturnStmt) stmtNode()              {}
func (s *ReturnStmt) GetSpan() parser.Span { return s.Span }

// BreakStmt exits the nearest loop in the typed tree.
type BreakStmt struct {
	Span parser.Span
}

func (*BreakStmt) stmtNode()              {}
func (s *BreakStmt) GetSpan() parser.Span { return s.Span }

// ExprStmt wraps a typed expression used for side effects.
type ExprStmt struct {
	Expr Expr
	Span parser.Span
}

func (*ExprStmt) stmtNode()              {}
func (s *ExprStmt) GetSpan() parser.Span { return s.Span }

// baseExpr provides common type/span behavior for typed expressions.
type baseExpr struct {
	Type *typecheck.Type
	Span parser.Span
}

func (e *baseExpr) GetType() *typecheck.Type { return e.Type }
func (e *baseExpr) GetSpan() parser.Span     { return e.Span }

// IdentifierExpr is a typed identifier reference.
type IdentifierExpr struct {
	baseExpr
	Name   string
	Symbol *SymbolRef
}

func (*IdentifierExpr) exprNode() {}

// PlaceholderExpr is a typed `_` placeholder expression.
type PlaceholderExpr struct {
	baseExpr
}

func (*PlaceholderExpr) exprNode() {}

// IntegerLiteral is a typed integer literal.
type IntegerLiteral struct {
	baseExpr
	Value string
}

func (*IntegerLiteral) exprNode() {}

// FloatLiteral is a typed floating-point literal.
type FloatLiteral struct {
	baseExpr
	Value string
}

func (*FloatLiteral) exprNode() {}

// RuneLiteral is a typed rune literal.
type RuneLiteral struct {
	baseExpr
	Value string
}

func (*RuneLiteral) exprNode() {}

// BoolLiteral is a typed boolean literal.
type BoolLiteral struct {
	baseExpr
	Value bool
}

func (*BoolLiteral) exprNode() {}

// StringLiteral is a typed string literal.
type StringLiteral struct {
	baseExpr
	Value string
}

func (*StringLiteral) exprNode() {}

// UnitLiteral is the typed unit value expression written as ().
type UnitLiteral struct {
	baseExpr
}

func (*UnitLiteral) exprNode() {}

// ListLiteral is a typed list literal.
type ListLiteral struct {
	baseExpr
	Elements []Expr
}

func (*ListLiteral) exprNode() {}

// TupleLiteral is a typed tuple literal.
type TupleLiteral struct {
	baseExpr
	Elements []Expr
}

func (*TupleLiteral) exprNode() {}

// AnonymousRecordExpr is a typed structural anonymous record literal.
type AnonymousRecordExpr struct {
	baseExpr
	Fields []RecordUpdateField
}

func (*AnonymousRecordExpr) exprNode() {}

// GroupExpr preserves a parenthesized typed expression.
type GroupExpr struct {
	baseExpr
	Inner Expr
}

func (*GroupExpr) exprNode() {}

// UnaryExpr is a typed unary operator application.
type UnaryExpr struct {
	baseExpr
	Operator string
	Right    Expr
}

func (*UnaryExpr) exprNode() {}

// BinaryExpr is a typed binary operator application.
type BinaryExpr struct {
	baseExpr
	Left     Expr
	Operator string
	Right    Expr
}

func (*BinaryExpr) exprNode() {}

// IsExpr is a typed runtime type-check expression.
type IsExpr struct {
	baseExpr
	Left   Expr
	Target *typecheck.Type
}

func (*IsExpr) exprNode() {}

// FieldExpr is a typed field read with an optional resolved field symbol.
type FieldExpr struct {
	baseExpr
	Receiver Expr
	Name     string
	Field    *SymbolRef
}

func (*FieldExpr) exprNode() {}

// IndexExpr is a typed indexing operation.
type IndexExpr struct {
	baseExpr
	Receiver Expr
	Index    Expr
}

func (*IndexExpr) exprNode() {}

// RecordUpdateField is a typed record field override in a copy/update expression.
type RecordUpdateField struct {
	Name  string
	Value Expr
}

// RecordUpdateExpr copies a record value and overrides selected fields.
type RecordUpdateExpr struct {
	baseExpr
	Receiver Expr
	Updates  []RecordUpdateField
}

func (*RecordUpdateExpr) exprNode() {}

// AnonymousInterfaceExpr is an inline object implementing one or more interfaces.
type AnonymousInterfaceExpr struct {
	baseExpr
	Interfaces []*typecheck.Type
}

func (*AnonymousInterfaceExpr) exprNode() {}

// IfExpr is a typed if / else expression.
type IfExpr struct {
	baseExpr
	Condition Expr
	Then      *BlockStmt
	Else      *BlockStmt
}

func (*IfExpr) exprNode() {}

// BlockExpr is a typed braced block expression.
type BlockExpr struct {
	baseExpr
	Body *BlockStmt
}

func (*BlockExpr) exprNode() {}

// MatchCase is a typed case in a match statement or expression.
type MatchCase struct {
	Pattern parser.Pattern
	Guard   Expr
	Body    *BlockStmt
	Expr    Expr
}

// MatchExpr is a typed match expression.
type MatchExpr struct {
	baseExpr
	Partial bool
	Value   Expr
	Cases   []MatchCase
}

func (*MatchExpr) exprNode() {}

// ForYieldExpr is a typed yield-style loop expression.
type ForYieldExpr struct {
	baseExpr
	Bindings  []ForBinding
	YieldBody *BlockStmt
}

func (*ForYieldExpr) exprNode() {}

// FunctionCallExpr is a typed call to a named top-level function.
type FunctionCallExpr struct {
	baseExpr
	Name     string
	Args     []Expr
	Function *SymbolRef
}

func (*FunctionCallExpr) exprNode() {}

// ConstructorCallExpr is a typed class construction call.
type ConstructorCallExpr struct {
	baseExpr
	Class       string
	Args        []Expr
	ClassSymbol *SymbolRef
	Constructor *SymbolRef
	Dispatch    CallDispatch
}

func (*ConstructorCallExpr) exprNode() {}

// MethodCallExpr is a typed method invocation on a receiver.
type MethodCallExpr struct {
	baseExpr
	Receiver Expr
	Method   string
	Args     []Expr
	Target   *SymbolRef
	Dispatch CallDispatch
}

func (*MethodCallExpr) exprNode() {}

// InvokeExpr is a typed call through a function value expression.
type InvokeExpr struct {
	baseExpr
	Callee Expr
	Args   []Expr
}

func (*InvokeExpr) exprNode() {}

// LambdaParameter is a typed lambda parameter with its symbol.
type LambdaParameter struct {
	Name   string
	Type   *typecheck.Type
	Symbol SymbolRef
	Span   parser.Span
}

// LambdaExpr is a typed lambda expression.
type LambdaExpr struct {
	baseExpr
	Parameters []LambdaParameter
	Body       Expr
	BlockBody  *BlockStmt
}

func (*LambdaExpr) exprNode() {}
