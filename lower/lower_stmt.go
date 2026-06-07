package lower

import (
	"fmt"

	"a-lang/typecheck"
	"a-lang/typed"
)

func unsupportedTopLevelErr(stmt typed.Stmt) error {
	return fmt.Errorf("unsupported top-level statement %T during lowering", stmt)
}

func unsupportedStmtErr(stmt typed.Stmt) error {
	return fmt.Errorf("unsupported statement %T during lowering", stmt)
}

func (l *Lowerer) lowerStmt(stmt typed.Stmt) ([]Stmt, error) {
	switch s := stmt.(type) {
	case *typed.BindingStmt:
		return l.lowerBindingStmt(s)
	case *typed.AssignmentStmt:
		return l.lowerAssignmentStmt(s)
	case *typed.IfStmt:
		return l.lowerIfStmt(s)
	case *typed.WhileStmt:
		return l.lowerWhileStmt(s)
	case *typed.ForStmt:
		return l.lowerForStmt(s)
	case *typed.ReturnStmt:
		return l.lowerReturnStmt(s)
	case *typed.BreakStmt:
		return l.lowerBreakStmt(s)
	case *typed.ExprStmt:
		return l.lowerExprStmt(s)
	default:
		return nil, unsupportedStmtErr(stmt)
	}
}

func (l *Lowerer) lowerBindingStmt(stmt *typed.BindingStmt) ([]Stmt, error) {
	var out []Stmt
	for _, binding := range stmt.Bindings {
		var init Expr
		var err error
		if binding.Init != nil {
			init, err = l.lowerExpr(binding.Init)
			if err != nil {
				return nil, err
			}
		}
		out = append(out, &VarDecl{
			Name:    binding.Name,
			Mutable: binding.Mode == typed.BindingMutable,
			Type:    binding.Type,
			Init:    init,
		})
	}
	return out, nil
}

func (l *Lowerer) lowerAssignmentStmt(stmt *typed.AssignmentStmt) ([]Stmt, error) {
	target, err := l.lowerExpr(stmt.Target)
	if err != nil {
		return nil, err
	}
	value, err := l.lowerExpr(stmt.Value)
	if err != nil {
		return nil, err
	}
	return []Stmt{&Assign{Target: target, Operator: stmt.Operator, Value: value}}, nil
}

func (l *Lowerer) lowerIfStmt(stmt *typed.IfStmt) ([]Stmt, error) {
	cond, err := l.lowerExpr(stmt.Condition)
	if err != nil {
		return nil, err
	}
	thenBlock, err := l.lowerBlock(stmt.Then)
	if err != nil {
		return nil, err
	}
	var elseBlock []Stmt
	if stmt.ElseIf != nil {
		branch, err := l.lowerStmt(stmt.ElseIf)
		if err != nil {
			return nil, err
		}
		elseBlock = branch
	} else if stmt.Else != nil {
		elseBlock, err = l.lowerBlock(stmt.Else)
		if err != nil {
			return nil, err
		}
	}
	return []Stmt{&If{Condition: cond, Then: thenBlock, Else: elseBlock}}, nil
}

func (l *Lowerer) lowerReturnStmt(stmt *typed.ReturnStmt) ([]Stmt, error) {
	value, err := l.lowerExpr(stmt.Value)
	if err != nil {
		return nil, err
	}
	return []Stmt{&Return{Value: value}}, nil
}

func (l *Lowerer) lowerBreakStmt(_ *typed.BreakStmt) ([]Stmt, error) {
	return []Stmt{&Break{}}, nil
}

func (l *Lowerer) lowerExprStmt(stmt *typed.ExprStmt) ([]Stmt, error) {
	expr, err := l.lowerExpr(stmt.Expr)
	if err != nil {
		return nil, err
	}
	return []Stmt{&ExprStmt{Expr: expr}}, nil
}

func (l *Lowerer) lowerWhileStmt(stmt *typed.WhileStmt) ([]Stmt, error) {
	cond, err := l.lowerExpr(stmt.Condition)
	if err != nil {
		return nil, err
	}
	body, err := l.lowerBlock(stmt.Body)
	if err != nil {
		return nil, err
	}
	return []Stmt{&While{Condition: cond, Body: body}}, nil
}

func (l *Lowerer) lowerForStmt(stmt *typed.ForStmt) ([]Stmt, error) {
	body, err := l.lowerBlock(stmt.Body)
	if err != nil {
		return nil, err
	}
	if stmt.YieldBody == nil {
		return l.lowerForBindings(stmt.Bindings, body)
	}

	yieldPrefix, yieldExpr, yieldType, err := l.lowerYieldBody(stmt.YieldBody)
	if err != nil {
		return nil, err
	}
	resultType := &typecheck.Type{Kind: typecheck.TypeBuiltin, Name: "List", Args: []*typecheck.Type{yieldType}}
	resultName := l.nextTemp("yield")
	resultRef := &VarRef{Name: resultName, Type: resultType}

	body = append(body, yieldPrefix...)
	body = append(body, &Assign{
		Target:   resultRef,
		Operator: ":=",
		Value: &BuiltinCall{
			Name: "append",
			Args: []Expr{resultRef, yieldExpr},
			Type: resultType,
		},
	})

	stmts := []Stmt{&VarDecl{
		Name:    resultName,
		Mutable: true,
		Type:    resultType,
		Init:    &ListLiteral{Elements: nil, Type: resultType},
	}}
	loops, err := l.lowerForBindings(stmt.Bindings, body)
	if err != nil {
		return nil, err
	}
	stmts = append(stmts, loops...)
	return stmts, nil
}

func (l *Lowerer) lowerForBindings(bindings []typed.ForBinding, body []Stmt) ([]Stmt, error) {
	out := body
	for i := len(bindings) - 1; i >= 0; i-- {
		if len(bindings[i].Values) > 0 {
			if len(bindings[i].Bindings) != 1 {
				return nil, fmt.Errorf("lowering only supports single local for bindings, got %d", len(bindings[i].Bindings))
			}
			binding := bindings[i].Bindings[0]
			var init Expr
			var err error
			if len(bindings[i].Values) > 0 && bindings[i].Values[0] != nil {
				init, err = l.lowerExpr(bindings[i].Values[0])
				if err != nil {
					return nil, err
				}
			}
			out = append([]Stmt{&VarDecl{
				Name:    binding.Name,
				Mutable: binding.Mode == typed.BindingMutable,
				Type:    binding.Type,
				Init:    init,
			}}, out...)
			continue
		}
		if len(bindings[i].Bindings) != 1 {
			return nil, fmt.Errorf("lowering only supports single for binding, got %d", len(bindings[i].Bindings))
		}
		iterable, err := l.lowerExpr(bindings[i].Iterable)
		if err != nil {
			return nil, err
		}
		out = []Stmt{&ForEach{
			Name:     bindings[i].Bindings[0].Name,
			Iterable: iterable,
			Body:     out,
		}}
	}
	return out, nil
}

func (l *Lowerer) lowerYieldBody(block *typed.BlockStmt) ([]Stmt, Expr, *typecheck.Type, error) {
	if block == nil || len(block.Statements) == 0 {
		return nil, nil, unknownType(), fmt.Errorf("yield body must end with an expression")
	}
	prefix := block.Statements[:len(block.Statements)-1]
	last := block.Statements[len(block.Statements)-1]
	exprStmt, ok := last.(*typed.ExprStmt)
	if !ok {
		return nil, nil, unknownType(), fmt.Errorf("yield body must end with an expression statement")
	}
	loweredPrefix, err := l.lowerStmtBlock(prefix)
	if err != nil {
		return nil, nil, unknownType(), err
	}
	yieldExpr, err := l.lowerExpr(exprStmt.Expr)
	if err != nil {
		return nil, nil, unknownType(), err
	}
	yieldType := exprStmt.Expr.GetType()
	if yieldType == nil {
		yieldType = unknownType()
	}
	return loweredPrefix, yieldExpr, yieldType, nil
}

func (l *Lowerer) lowerValueBlock(block *typed.BlockStmt, emptyMessage string) ([]Stmt, Expr, error) {
	if block == nil || len(block.Statements) == 0 {
		return nil, nil, fmt.Errorf("%s", emptyMessage)
	}
	prefix := block.Statements[:len(block.Statements)-1]
	last := block.Statements[len(block.Statements)-1]
	exprStmt, ok := last.(*typed.ExprStmt)
	if !ok {
		return nil, nil, fmt.Errorf("%s", emptyMessage)
	}
	loweredPrefix, err := l.lowerStmtBlock(prefix)
	if err != nil {
		return nil, nil, err
	}
	value, err := l.lowerExpr(exprStmt.Expr)
	if err != nil {
		return nil, nil, err
	}
	return loweredPrefix, value, nil
}

func unwrapElemType(t *typecheck.Type) *typecheck.Type {
	if t == nil || len(t.Args) == 0 {
		return unknownType()
	}
	switch t.Name {
	case "Option", "Result":
		return t.Args[0]
	case "Either":
		if len(t.Args) > 1 {
			return t.Args[1]
		}
	}
	return unknownType()
}
