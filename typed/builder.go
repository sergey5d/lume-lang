package typed

import (
	"a-lang/parser"
	"a-lang/typecheck"
)

// Builder converts parser-level input into a typed semantic output.
type Builder[T any, R any] interface {
	Build(T) (R, error)
}

// buildContext stores the shared state that multiple typed-builder pieces need.
type buildContext struct {
	exprTypes        map[parser.Expr]*typecheck.Type
	exprAliases      map[parser.Expr]parser.Expr
	functions        map[string]*parser.FunctionDecl
	classes          map[string]*parser.ClassDecl
	interfaces       map[string]*parser.InterfaceDecl
	nextID           int
	scopes           []map[string]SymbolRef
	thisStack        []SymbolRef
	functionSymbols  map[string]SymbolRef
	classSymbols     map[string]SymbolRef
	interfaceSymbols map[string]SymbolRef
	fieldSymbols     map[string]map[string]SymbolRef
	methodSymbols    map[string]map[string][]methodSymbol
}

// methodSymbol pairs a method declaration with its stable typed symbol.
type methodSymbol struct {
	decl   *parser.MethodDecl
	symbol SymbolRef
}

// Build creates a typed program from the parser AST and type-checker result.
func Build(program *parser.Program, info typecheck.Result) (*Program, error) {
	ctx := newBuildContext(program, info)

	symbols := &symbolCollector{ctx: ctx}
	typeRefs := &typeRefBuilder{ctx: ctx}
	params := &parameterBuilder{ctx: ctx, types: typeRefs}

	blocks := &blockBuilder{ctx: ctx}
	exprs := &exprBuilder{ctx: ctx, types: typeRefs, blocks: blocks}
	calls := &callExprBuilder{ctx: ctx, exprs: exprs, types: typeRefs}
	lambdas := &lambdaExprBuilder{ctx: ctx, exprs: exprs, blocks: blocks, types: typeRefs}
	exprs.calls = calls
	exprs.lambdas = lambdas
	stmts := &stmtBuilder{
		bindings:         &bindingStmtBuilder{ctx: ctx, exprs: exprs, types: typeRefs},
		localFunctions:   &localFunctionStmtBuilder{ctx: ctx, blocks: blocks, types: typeRefs},
		assignments:      &assignmentStmtBuilder{exprs: exprs},
		multiAssignments: &multiAssignmentStmtBuilder{exprs: exprs},
		ifs:              &ifStmtBuilder{ctx: ctx, exprs: exprs, blocks: blocks, types: typeRefs},
		whiles:           &whileStmtBuilder{exprs: exprs, blocks: blocks},
		fors:             &forStmtBuilder{ctx: ctx, exprs: exprs, blocks: blocks, types: typeRefs},
		returns:          &returnStmtBuilder{exprs: exprs},
		breaks:           &breakStmtBuilder{},
		continues:        &continueStmtBuilder{},
		exprs:            &exprStmtBuilder{exprs: exprs},
	}
	blocks.stmts = stmts

	functions := &functionBuilder{ctx: ctx, params: params, blocks: blocks, types: typeRefs}
	interfaces := &interfaceBuilder{ctx: ctx, params: params, types: typeRefs}
	classes := &classBuilder{ctx: ctx, exprs: exprs, blocks: blocks, params: params, types: typeRefs}
	programs := &programBuilder{
		ctx:        ctx,
		stmts:      stmts,
		functions:  functions,
		interfaces: interfaces,
		classes:    classes,
	}

	symbols.collect(program)
	return programs.Build(program)
}

// newBuildContext indexes declarations and initializes shared typed-builder state.
func newBuildContext(program *parser.Program, info typecheck.Result) *buildContext {
	ctx := &buildContext{
		exprTypes:        info.ExprTypes,
		exprAliases:      info.ExprAliases,
		functions:        map[string]*parser.FunctionDecl{},
		classes:          map[string]*parser.ClassDecl{},
		interfaces:       map[string]*parser.InterfaceDecl{},
		functionSymbols:  map[string]SymbolRef{},
		classSymbols:     map[string]SymbolRef{},
		interfaceSymbols: map[string]SymbolRef{},
		fieldSymbols:     map[string]map[string]SymbolRef{},
		methodSymbols:    map[string]map[string][]methodSymbol{},
	}
	for _, fn := range program.Functions {
		ctx.functions[fn.Name] = fn
	}
	for _, class := range program.Classes {
		ctx.classes[class.Name] = class
	}
	for _, iface := range program.Interfaces {
		ctx.interfaces[iface.Name] = iface
	}
	return ctx
}

// newSymbol allocates a fresh stable symbol identifier.
func (c *buildContext) newSymbol(kind SymbolKind, name, owner string, span parser.Span) SymbolRef {
	c.nextID++
	return SymbolRef{ID: c.nextID, Kind: kind, Name: name, Owner: owner, Span: span}
}

// pushScope starts a new lexical symbol scope for typed building.
func (c *buildContext) pushScope() {
	c.scopes = append(c.scopes, map[string]SymbolRef{})
}

// popScope exits the current lexical symbol scope.
func (c *buildContext) popScope() {
	c.scopes = c.scopes[:len(c.scopes)-1]
}

// defineSymbol adds a symbol to the current lexical scope.
func (c *buildContext) defineSymbol(symbol SymbolRef) {
	if len(c.scopes) == 0 {
		c.pushScope()
	}
	c.scopes[len(c.scopes)-1][symbol.Name] = symbol
}

// lookupSymbol resolves a name through lexical scopes and top-level declarations.
func (c *buildContext) lookupSymbol(name string) (*SymbolRef, bool) {
	for i := len(c.scopes) - 1; i >= 0; i-- {
		if symbol, ok := c.scopes[i][name]; ok {
			return &symbol, true
		}
	}
	if symbol, ok := c.functionSymbols[name]; ok {
		return &symbol, true
	}
	if symbol, ok := c.classSymbols[name]; ok {
		return &symbol, true
	}
	if symbol, ok := c.interfaceSymbols[name]; ok {
		return &symbol, true
	}
	return nil, false
}
