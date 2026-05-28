package parser

func isSingleArgFunctionType(ref *TypeRef) bool {
	return ref != nil && ref.ReturnType != nil && len(ref.ParameterTypes) == 1
}

func HasPlaceholderExpr(expr Expr) bool {
	switch e := expr.(type) {
	case *PlaceholderExpr:
		return true
	case *LambdaExpr:
		return false
	case *GroupExpr:
		return HasPlaceholderExpr(e.Inner)
	case *UnaryExpr:
		return HasPlaceholderExpr(e.Right)
	case *BinaryExpr:
		return HasPlaceholderExpr(e.Left) || HasPlaceholderExpr(e.Right)
	case *IsExpr:
		return HasPlaceholderExpr(e.Left)
	case *CallExpr:
		if HasPlaceholderExpr(e.Callee) {
			return true
		}
		for _, arg := range e.Args {
			if HasPlaceholderExpr(arg.Value) {
				return true
			}
		}
		return false
	case *MemberExpr:
		return HasPlaceholderExpr(e.Receiver)
	case *IndexExpr:
		return HasPlaceholderExpr(e.Receiver) || HasPlaceholderExpr(e.Index)
	case *RecordUpdateExpr:
		if HasPlaceholderExpr(e.Receiver) {
			return true
		}
		for _, update := range e.Updates {
			if HasPlaceholderExpr(update.Value) {
				return true
			}
		}
		return false
	case *AnonymousRecordExpr:
		if len(e.Values) > 0 {
			for _, value := range e.Values {
				if HasPlaceholderExpr(value) {
					return true
				}
			}
		}
		for _, field := range e.Fields {
			if HasPlaceholderExpr(field.Value) {
				return true
			}
		}
		return false
	case *AnonymousInterfaceExpr:
		for _, method := range e.Methods {
			if blockHasPlaceholder(method.Body) {
				return true
			}
		}
		return false
	case *ListLiteral:
		for _, item := range e.Elements {
			if HasPlaceholderExpr(item) {
				return true
			}
		}
		return false
	case *TupleLiteral:
		for _, item := range e.Elements {
			if HasPlaceholderExpr(item) {
				return true
			}
		}
		return false
	case *IfExpr:
		return HasPlaceholderExpr(e.Condition) || blockHasPlaceholder(e.Then) || blockHasPlaceholder(e.Else)
	case *BlockExpr:
		return blockHasPlaceholder(e.Body)
	case *MatchExpr:
		if HasPlaceholderExpr(e.Value) {
			return true
		}
		for _, matchCase := range e.Cases {
			if matchCase.Guard != nil && HasPlaceholderExpr(matchCase.Guard) {
				return true
			}
			if matchCase.Expr != nil && HasPlaceholderExpr(matchCase.Expr) {
				return true
			}
			if blockHasPlaceholder(matchCase.Body) {
				return true
			}
		}
		return false
	case *ForYieldExpr:
		for _, binding := range e.Bindings {
			if HasPlaceholderExpr(binding.Iterable) {
				return true
			}
			for _, value := range binding.Values {
				if HasPlaceholderExpr(value) {
					return true
				}
			}
		}
		return blockHasPlaceholder(e.YieldBody)
	default:
		return false
	}
}

func blockHasPlaceholder(block *BlockStmt) bool {
	if block == nil {
		return false
	}
	for _, stmt := range block.Statements {
		if stmtHasPlaceholder(stmt) {
			return true
		}
	}
	return false
}

func stmtHasPlaceholder(stmt Statement) bool {
	switch s := stmt.(type) {
	case *ValStmt:
		for _, value := range s.Values {
			if HasPlaceholderExpr(value) {
				return true
			}
		}
	case *AssignmentStmt:
		return HasPlaceholderExpr(s.Target) || HasPlaceholderExpr(s.Value)
	case *MultiAssignmentStmt:
		for _, target := range s.Targets {
			if HasPlaceholderExpr(target) {
				return true
			}
		}
		for _, value := range s.Values {
			if HasPlaceholderExpr(value) {
				return true
			}
		}
	case *IfStmt:
		if HasPlaceholderExpr(s.Condition) || HasPlaceholderExpr(s.PatternValue) || HasPlaceholderExpr(s.BindingValue) || blockHasPlaceholder(s.Then) || blockHasPlaceholder(s.Else) {
			return true
		}
		for _, clause := range s.PatternClauses {
			if HasPlaceholderExpr(clause.Value) {
				return true
			}
		}
		if s.ElseIf != nil {
			return stmtHasPlaceholder(s.ElseIf)
		}
	case *MatchStmt:
		if HasPlaceholderExpr(s.Value) {
			return true
		}
		for _, matchCase := range s.Cases {
			if matchCase.Guard != nil && HasPlaceholderExpr(matchCase.Guard) {
				return true
			}
			if matchCase.Expr != nil && HasPlaceholderExpr(matchCase.Expr) {
				return true
			}
			if blockHasPlaceholder(matchCase.Body) {
				return true
			}
		}
	case *ForStmt:
		if blockHasPlaceholder(s.Body) || blockHasPlaceholder(s.YieldBody) {
			return true
		}
		for _, binding := range s.Bindings {
			if HasPlaceholderExpr(binding.Iterable) {
				return true
			}
			for _, value := range binding.Values {
				if HasPlaceholderExpr(value) {
					return true
				}
			}
		}
	case *WhileStmt:
		if HasPlaceholderExpr(s.Condition) || blockHasPlaceholder(s.Body) {
			return true
		}
	case *ReturnStmt:
		return HasPlaceholderExpr(s.Value)
	case *ExprStmt:
		return HasPlaceholderExpr(s.Expr)
	}
	return false
}

func WrapContextualFunctionExpr(ref *TypeRef, expr Expr) Expr {
	if ref == nil {
		return expr
	}
	if _, ok := expr.(*LambdaExpr); ok {
		return expr
	}
	if isZeroArgFunctionType(ref) {
		return &LambdaExpr{
			Parameters: []LambdaParameter{},
			Body:       expr,
			Span:       exprSpan(expr),
		}
	}
	if isSingleArgFunctionType(ref) && HasPlaceholderExpr(expr) {
		return WrapPlaceholderLambdaExpr(expr)
	}
	return expr
}

func WrapPlaceholderLambdaExpr(expr Expr) Expr {
	if _, ok := expr.(*LambdaExpr); ok {
		return expr
	}
	if !HasPlaceholderExpr(expr) {
		return expr
	}
	param := "v1"
	return &LambdaExpr{
		Parameters: []LambdaParameter{{Name: param, Span: exprSpan(expr)}},
		Body:       replacePlaceholderExpr(expr, param),
		Span:       exprSpan(expr),
	}
}

func replacePlaceholderExpr(expr Expr, param string) Expr {
	switch e := expr.(type) {
	case *PlaceholderExpr:
		return &Identifier{Name: param, Span: e.Span}
	case *LambdaExpr:
		return e
	case *GroupExpr:
		return &GroupExpr{Inner: replacePlaceholderExpr(e.Inner, param), Span: e.Span}
	case *UnaryExpr:
		return &UnaryExpr{Operator: e.Operator, Right: replacePlaceholderExpr(e.Right, param), Span: e.Span}
	case *BinaryExpr:
		return &BinaryExpr{
			Left:     replacePlaceholderExpr(e.Left, param),
			Operator: e.Operator,
			Right:    replacePlaceholderExpr(e.Right, param),
			Span:     e.Span,
		}
	case *IsExpr:
		return &IsExpr{Left: replacePlaceholderExpr(e.Left, param), Target: e.Target, Span: e.Span}
	case *CallExpr:
		args := make([]CallArg, len(e.Args))
		for i, arg := range e.Args {
			args[i] = CallArg{Name: arg.Name, Value: replacePlaceholderExpr(arg.Value, param), Span: arg.Span}
		}
		return &CallExpr{Callee: replacePlaceholderExpr(e.Callee, param), Args: args, Span: e.Span}
	case *MemberExpr:
		return &MemberExpr{Receiver: replacePlaceholderExpr(e.Receiver, param), Name: e.Name, Span: e.Span}
	case *IndexExpr:
		return &IndexExpr{Receiver: replacePlaceholderExpr(e.Receiver, param), Index: replacePlaceholderExpr(e.Index, param), Span: e.Span}
	case *RecordUpdateExpr:
		updates := make([]CallArg, len(e.Updates))
		for i, update := range e.Updates {
			updates[i] = CallArg{Name: update.Name, Value: replacePlaceholderExpr(update.Value, param), Span: update.Span}
		}
		return &RecordUpdateExpr{Receiver: replacePlaceholderExpr(e.Receiver, param), Updates: updates, Span: e.Span}
	case *AnonymousRecordExpr:
		values := make([]Expr, len(e.Values))
		for i, value := range e.Values {
			values[i] = replacePlaceholderExpr(value, param)
		}
		fields := make([]CallArg, len(e.Fields))
		for i, field := range e.Fields {
			fields[i] = CallArg{Name: field.Name, Value: replacePlaceholderExpr(field.Value, param), Span: field.Span}
		}
		return &AnonymousRecordExpr{Fields: fields, Values: values, Span: e.Span}
	case *AnonymousInterfaceExpr:
		methods := make([]*MethodDecl, len(e.Methods))
		for i, method := range e.Methods {
			copyMethod := *method
			copyMethod.Body = replacePlaceholderBlock(method.Body, param)
			methods[i] = &copyMethod
		}
		return &AnonymousInterfaceExpr{Interfaces: append([]*TypeRef(nil), e.Interfaces...), Methods: methods, Span: e.Span}
	case *ListLiteral:
		items := make([]Expr, len(e.Elements))
		for i, item := range e.Elements {
			items[i] = replacePlaceholderExpr(item, param)
		}
		return &ListLiteral{Elements: items, Span: e.Span}
	case *TupleLiteral:
		items := make([]Expr, len(e.Elements))
		for i, item := range e.Elements {
			items[i] = replacePlaceholderExpr(item, param)
		}
		return &TupleLiteral{Elements: items, Span: e.Span}
	case *IfExpr:
		return &IfExpr{
			Condition: replacePlaceholderExpr(e.Condition, param),
			Then:      replacePlaceholderBlock(e.Then, param),
			Else:      replacePlaceholderBlock(e.Else, param),
			Span:      e.Span,
		}
	case *BlockExpr:
		return &BlockExpr{Body: replacePlaceholderBlock(e.Body, param), Span: e.Span}
	case *MatchExpr:
		cases := make([]MatchCase, len(e.Cases))
		for i, matchCase := range e.Cases {
			cases[i] = MatchCase{
				Pattern: matchCase.Pattern,
				Guard:   replacePlaceholderExpr(matchCase.Guard, param),
				Body:    replacePlaceholderBlock(matchCase.Body, param),
				Expr:    replacePlaceholderExpr(matchCase.Expr, param),
				Span:    matchCase.Span,
			}
		}
		return &MatchExpr{Partial: e.Partial, Value: replacePlaceholderExpr(e.Value, param), Cases: cases, Span: e.Span}
	case *ForYieldExpr:
		bindings := make([]ForBinding, len(e.Bindings))
		for i, binding := range e.Bindings {
			values := make([]Expr, len(binding.Values))
			for j, value := range binding.Values {
				values[j] = replacePlaceholderExpr(value, param)
			}
			bindings[i] = ForBinding{
				Bindings: binding.Bindings,
				Iterable: replacePlaceholderExpr(binding.Iterable, param),
				Values:   values,
				Span:     binding.Span,
			}
		}
		return &ForYieldExpr{Bindings: bindings, YieldBody: replacePlaceholderBlock(e.YieldBody, param), Span: e.Span}
	default:
		return expr
	}
}

func replacePlaceholderBlock(block *BlockStmt, param string) *BlockStmt {
	if block == nil {
		return nil
	}
	statements := make([]Statement, len(block.Statements))
	for i, stmt := range block.Statements {
		statements[i] = replacePlaceholderStmt(stmt, param)
	}
	return &BlockStmt{Statements: statements, Span: block.Span}
}

func replacePlaceholderStmt(stmt Statement, param string) Statement {
	switch s := stmt.(type) {
	case *ValStmt:
		values := make([]Expr, len(s.Values))
		for i, value := range s.Values {
			values[i] = replacePlaceholderExpr(value, param)
		}
		return &ValStmt{Bindings: append([]Binding(nil), s.Bindings...), Values: values, Span: s.Span}
	case *AssignmentStmt:
		return &AssignmentStmt{Target: replacePlaceholderExpr(s.Target, param), Operator: s.Operator, Value: replacePlaceholderExpr(s.Value, param), Span: s.Span}
	case *MultiAssignmentStmt:
		targets := make([]Expr, len(s.Targets))
		for i, target := range s.Targets {
			targets[i] = replacePlaceholderExpr(target, param)
		}
		values := make([]Expr, len(s.Values))
		for i, value := range s.Values {
			values[i] = replacePlaceholderExpr(value, param)
		}
		return &MultiAssignmentStmt{Targets: targets, Operator: s.Operator, Values: values, Span: s.Span}
	case *IfStmt:
		var elseIf *IfStmt
		if s.ElseIf != nil {
			elseIf = replacePlaceholderStmt(s.ElseIf, param).(*IfStmt)
		}
		patternClauses := make([]RefutableClause, len(s.PatternClauses))
		for i, clause := range s.PatternClauses {
			patternClauses[i] = RefutableClause{
				Pattern: clause.Pattern,
				Value:   replacePlaceholderExpr(clause.Value, param),
				Span:    clause.Span,
			}
		}
		return &IfStmt{
			Condition:      replacePlaceholderExpr(s.Condition, param),
			Pattern:        s.Pattern,
			PatternValue:   replacePlaceholderExpr(s.PatternValue, param),
			PatternClauses: patternClauses,
			Bindings:       append([]Binding(nil), s.Bindings...),
			BindingValue:   replacePlaceholderExpr(s.BindingValue, param),
			Then:           replacePlaceholderBlock(s.Then, param),
			ElseIf:         elseIf,
			Else:           replacePlaceholderBlock(s.Else, param),
			Span:           s.Span,
		}
	case *MatchStmt:
		cases := make([]MatchCase, len(s.Cases))
		for i, matchCase := range s.Cases {
			cases[i] = MatchCase{
				Pattern: matchCase.Pattern,
				Guard:   replacePlaceholderExpr(matchCase.Guard, param),
				Body:    replacePlaceholderBlock(matchCase.Body, param),
				Expr:    replacePlaceholderExpr(matchCase.Expr, param),
				Span:    matchCase.Span,
			}
		}
		return &MatchStmt{Partial: s.Partial, Value: replacePlaceholderExpr(s.Value, param), Cases: cases, Span: s.Span}
	case *ForStmt:
		bindings := make([]ForBinding, len(s.Bindings))
		for i, binding := range s.Bindings {
			values := make([]Expr, len(binding.Values))
			for j, value := range binding.Values {
				values[j] = replacePlaceholderExpr(value, param)
			}
			bindings[i] = ForBinding{
				Bindings: binding.Bindings,
				Iterable: replacePlaceholderExpr(binding.Iterable, param),
				Values:   values,
				Span:     binding.Span,
			}
		}
		return &ForStmt{
			Bindings:  bindings,
			Body:      replacePlaceholderBlock(s.Body, param),
			YieldBody: replacePlaceholderBlock(s.YieldBody, param),
			Span:      s.Span,
		}
	case *WhileStmt:
		return &WhileStmt{
			Condition: replacePlaceholderExpr(s.Condition, param),
			Body:      replacePlaceholderBlock(s.Body, param),
			Span:      s.Span,
		}
	case *ReturnStmt:
		return &ReturnStmt{Value: replacePlaceholderExpr(s.Value, param), Span: s.Span}
	case *ExprStmt:
		return &ExprStmt{Expr: replacePlaceholderExpr(s.Expr, param), Span: s.Span}
	default:
		return stmt
	}
}
