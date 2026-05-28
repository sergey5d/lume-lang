package parser

import (
	"fmt"
	"strings"
)

func (p *Parser) parseBlock() (*BlockStmt, error) {
	start, err := p.consume(TokenLBrace, "expected '{'")
	if err != nil {
		return nil, err
	}
	return p.parseBlockAfterStart(start)
}

func (p *Parser) parseBlockAfterStart(start Token) (*BlockStmt, error) {
	p.beginScope()
	defer p.endScope()
	block := &BlockStmt{}
	for !p.check(TokenRBrace) && !p.isAtEnd() {
		stmt, err := p.parseStatement()
		if err != nil {
			return nil, err
		}
		block.Statements = append(block.Statements, stmt)
	}
	end, err := p.consume(TokenRBrace, "expected '}'")
	if err != nil {
		return nil, err
	}
	block.Span = mergeSpans(tokenSpan(start), tokenSpan(end))
	return block, nil
}

func (p *Parser) parseStatement() (Statement, error) {
	switch p.peek().Type {
	case TokenIf:
		return p.parseIfStmt()
	case TokenPartial:
		return p.parsePartialStmt()
	case TokenMatch:
		return p.parseMatchStmt()
	case TokenWhile:
		return p.parseWhileStmt()
	case TokenFor:
		return p.parseForStmt()
	case TokenDef:
		return p.parseLocalFunctionStmt()
	case TokenVar:
		return p.parseVarBindingStmt()
	case TokenLet:
		return p.parseLetStmt()
	case TokenReturn:
		return p.parseReturnStmt()
	case TokenBreak:
		token := p.advance()
		return &BreakStmt{Span: tokenSpan(token)}, nil
	default:
		if p.isUnwrapStmtStart() {
			return p.parseUnwrapStmt()
		}
		if p.isBareBindingStart() {
			return p.parseBareBindingStmt()
		}
		return p.parseExprStmt()
	}
}

func (p *Parser) parseLocalFunctionStmt() (Statement, error) {
	fn, err := p.parseFunction()
	if err != nil {
		return nil, err
	}
	return &LocalFunctionStmt{Function: fn, Span: fn.Span}, nil
}

func (p *Parser) parseBareBindingStmt() (Statement, error) {
	var start Token
	var err error
	if p.check(TokenIdentifier) {
		start, err = p.consume(TokenIdentifier, "expected binding name")
	} else {
		start, err = p.consume(TokenUnder, "expected binding name")
	}
	if err != nil {
		return nil, err
	}
	return p.parseBindingStmtWithStart(start, true)
}

func (p *Parser) parseVarBindingStmt() (Statement, error) {
	if _, err := p.consume(TokenVar, "expected 'var'"); err != nil {
		return nil, err
	}
	var start Token
	var err error
	if p.check(TokenIdentifier) {
		start, err = p.consume(TokenIdentifier, "expected binding name after 'var'")
	} else {
		start, err = p.consume(TokenUnder, "expected binding name after 'var'")
	}
	if err != nil {
		return nil, err
	}
	bindings, err := p.parseBindingsWithStart(start, true)
	if err != nil {
		return nil, err
	}
	if !p.check(TokenAssign) {
		if p.check(TokenColonAssign) {
			return nil, fmt.Errorf("cannot use ':=' in a declaration; use 'var' with '=' for mutable bindings")
		}
		return nil, fmt.Errorf("expected '=' after 'var' bindings, got %s", p.peek().String())
	}
	operator := p.advance()
	for i := range bindings {
		bindings[i].Mutable = true
	}
	values, err := p.parseBindingInitializers(len(bindings), operator)
	if err != nil {
		return nil, err
	}
	if len(bindings) > 1 && len(values) == 1 {
		return nil, fmt.Errorf("destructuring bindings require 'let (...) = value'")
	}
	for i := range bindings {
		if i >= len(values) || values[i] == nil {
			return nil, fmt.Errorf("mutable declarations must initialize with '='; deferred '?' initializers are not supported")
		}
		values[i] = wrapThunkExpr(bindings[i].Type, values[i])
	}
	stmt := &ValStmt{Bindings: bindings, Values: values}
	stmt.Span = tokenSpan(start)
	for i := range bindings {
		stmt.Span = mergeSpans(stmt.Span, exprSpan(values[i]))
	}
	for _, binding := range bindings {
		if binding.Name != "_" {
			p.declare(binding.Name)
		}
	}
	return stmt, nil
}

func (p *Parser) parseLetStmt() (Statement, error) {
	start, err := p.consume(TokenLet, "expected 'let'")
	if err != nil {
		return nil, err
	}

	if p.check(TokenLBrace) {
		clauses, span, err := p.parseRefutableClauseBlock("let")
		if err != nil {
			return nil, err
		}
		if !p.match(TokenElse) {
			return nil, fmt.Errorf("expected 'else' after let clause block")
		}
		elseToken := p.previous()
		fallback, err := p.parseBlockOrInlineStmtBody(elseToken, "let else")
		if err != nil {
			return nil, err
		}
		return &LetElseStmt{
			Clauses:  clauses,
			Fallback: fallback,
			Span:     mergeSpans(tokenSpan(start), mergeSpans(span, fallback.Span)),
		}, nil
	}

	if p.match(TokenLParen) {
		var start Token
		var err error
		if p.check(TokenIdentifier) {
			start, err = p.consume(TokenIdentifier, "expected binding name after '('")
		} else {
			start, err = p.consume(TokenUnder, "expected binding name after '('")
		}
		if err != nil {
			return nil, err
		}
		bindings, err := p.parseBindingsWithStart(start, true)
		if err != nil {
			return nil, err
		}
		if _, err := p.consume(TokenRParen, "expected ')' after destructuring bindings"); err != nil {
			return nil, err
		}
		operator, err := p.consume(TokenAssign, "expected '=' after destructuring bindings")
		if err != nil {
			return nil, err
		}
		values, err := p.parseBindingInitializers(len(bindings), operator)
		if err != nil {
			return nil, err
		}
		if len(values) != 1 || values[0] == nil {
			return nil, fmt.Errorf("destructuring bindings require a single initializer expression")
		}
		value := wrapThunkExpr(nil, values[0])
		stmt := &ValStmt{Bindings: bindings, Values: []Expr{value}}
		stmt.Span = mergeSpans(tokenSpan(start), exprSpan(value))
		for _, binding := range bindings {
			if binding.Name != "_" {
				p.declare(binding.Name)
			}
		}
		return stmt, nil
	}

	save := p.pos
	if (p.check(TokenIdentifier) && !p.checkNext(TokenLParen) && !p.checkNext(TokenDot)) || p.check(TokenUnder) {
		var start Token
		var err error
		if p.check(TokenIdentifier) {
			start, err = p.consume(TokenIdentifier, "expected binding name after 'let'")
		} else {
			start, err = p.consume(TokenUnder, "expected binding name after 'let'")
		}
		if err != nil {
			return nil, err
		}
		if stmt, err := p.parseBindingStmtWithStart(start, true); err == nil {
			return stmt, nil
		}
		p.pos = save
	}

	pattern, err := p.parsePattern()
	if err != nil {
		return nil, err
	}
	if _, err := p.consume(TokenAssign, "expected '=' after let pattern"); err != nil {
		return nil, err
	}
	if err := p.requireSameLineExpressionStart(p.previous()); err != nil {
		return nil, err
	}
	value, err := p.parseInlineExpression(TokenElse)
	if err != nil {
		return nil, err
	}
	if !p.match(TokenElse) {
		return nil, fmt.Errorf("expected 'else' after let pattern")
	}
	elseToken := p.previous()
	fallback, err := p.parseBlockOrInlineStmtBody(elseToken, "let else")
	if err != nil {
		return nil, err
	}
	return &LetElseStmt{
		Pattern:  pattern,
		Value:    value,
		Fallback: fallback,
		Span:     mergeSpans(patternSpan(pattern), fallback.Span),
	}, nil
}

func (p *Parser) parseRefutableClause(owner string) (RefutableClause, error) {
	pattern, err := p.parsePattern()
	if err != nil {
		return RefutableClause{}, err
	}
	if _, err := p.consume(TokenAssign, "expected '=' after "+owner+" pattern"); err != nil {
		return RefutableClause{}, err
	}
	if err := p.requireSameLineExpressionStart(p.previous()); err != nil {
		return RefutableClause{}, err
	}
	value, err := p.parseInlineExpression(TokenRBrace)
	if err != nil {
		return RefutableClause{}, err
	}
	return RefutableClause{
		Pattern: pattern,
		Value:   value,
		Span:    mergeSpans(patternSpan(pattern), exprSpan(value)),
	}, nil
}

func (p *Parser) parseRefutableClauseBlock(owner string) ([]RefutableClause, Span, error) {
	open, err := p.consume(TokenLBrace, "expected '{' after "+owner)
	if err != nil {
		return nil, Span{}, err
	}
	var clauses []RefutableClause
	for !p.check(TokenRBrace) && !p.isAtEnd() {
		clause, err := p.parseRefutableClause(owner)
		if err != nil {
			return nil, Span{}, err
		}
		clauses = append(clauses, clause)
	}
	if len(clauses) == 0 {
		return nil, Span{}, fmt.Errorf("%s clause block must contain at least one 'PATTERN = value' clause", owner)
	}
	close, err := p.consume(TokenRBrace, "expected '}' after "+owner+" clause block")
	if err != nil {
		return nil, Span{}, err
	}
	return clauses, mergeSpans(tokenSpan(open), tokenSpan(close)), nil
}

func (p *Parser) parseBindingStmtWithStart(start Token, firstIsName bool) (Statement, error) {
	bindings, err := p.parseBindingsWithStart(start, firstIsName)
	if err != nil {
		return nil, err
	}

	operator := p.peek().Type
	if operator != TokenAssign && operator != TokenColonAssign && operator != TokenLeftArrow {
		return nil, fmt.Errorf("expected '=', ':=', or '<-' after bindings, got %s", p.peek().String())
	}
	if operator == TokenColonAssign {
		return nil, fmt.Errorf("cannot use ':=' in a declaration; use 'var' with '=' for mutable bindings")
	}
	p.advance()
	if operator == TokenLeftArrow {
		if p.unwrapBlockDepth == 0 {
			return nil, fmt.Errorf("unwrap binding requires 'unwrap', 'if', or 'for' context")
		}
		if err := p.requireSameLineExpressionStart(p.previous()); err != nil {
			return nil, err
		}
		value, err := p.parseInlineExpression(TokenElse)
		if err != nil {
			return nil, err
		}
		stmt := &UnwrapStmt{Bindings: bindings, Value: value}
		stmt.Span = mergeSpans(tokenSpan(start), exprSpan(value))
		return stmt, nil
	}
	values, err := p.parseBindingInitializers(len(bindings), p.previous())
	if err != nil {
		return nil, err
	}
	if len(bindings) > 1 && len(values) == 1 {
		return nil, fmt.Errorf("destructuring bindings require 'let (...) = value'")
	}
	for i := range bindings {
		if i < len(values) && values[i] != nil {
			values[i] = wrapThunkExpr(bindings[i].Type, values[i])
		}
	}
	stmt := &ValStmt{Bindings: bindings, Values: values}
	stmt.Span = tokenSpan(start)
	for i := range bindings {
		if i < len(values) && values[i] != nil {
			stmt.Span = mergeSpans(stmt.Span, exprSpan(values[i]))
			continue
		}
		if i >= len(values) {
			continue
		}
		bindings[i].Deferred = true
		stmt.Bindings[i].Deferred = true
		if i == len(bindings)-1 {
			stmt.Span = mergeSpans(stmt.Span, bindings[i].Span)
		}
	}
	for _, binding := range bindings {
		if binding.Name != "_" {
			p.declare(binding.Name)
		}
	}
	return stmt, nil
}

func (p *Parser) parseUnwrapStmt() (Statement, error) {
	start, err := p.consume(TokenIdentifier, "expected 'unwrap'")
	if err != nil {
		return nil, err
	}
	if start.Lexeme != "unwrap" {
		return nil, fmt.Errorf("expected 'unwrap'")
	}
	if p.check(TokenLBrace) {
		return p.parseUnwrapBlockStmt(start)
	}
	var name Token
	if p.check(TokenIdentifier) {
		name, err = p.consume(TokenIdentifier, "expected binding name after 'unwrap'")
	} else {
		name, err = p.consume(TokenUnder, "expected binding name after 'unwrap'")
	}
	if err != nil {
		return nil, err
	}
	bindings, err := p.parseBindingsWithStart(name, true)
	if err != nil {
		return nil, err
	}
	if _, err := p.consume(TokenLeftArrow, "expected '<-' after unwrap bindings"); err != nil {
		return nil, err
	}
	if err := p.requireSameLineExpressionStart(p.previous()); err != nil {
		return nil, err
	}
	value, err := p.parseInlineExpression(TokenElse)
	if err != nil {
		return nil, err
	}
	if p.match(TokenElse) {
		elseToken := p.previous()
		fallback, err := p.parseBlockOrInlineStmtBody(elseToken, "unwrap else")
		if err != nil {
			return nil, err
		}
		return &GuardStmt{
			Bindings: bindings,
			Value:    value,
			Fallback: fallback,
			Span:     mergeSpans(tokenSpan(start), fallback.Span),
		}, nil
	}
	return &UnwrapStmt{
		Bindings: bindings,
		Value:    value,
		Span:     mergeSpans(tokenSpan(start), exprSpan(value)),
	}, nil
}

func (p *Parser) isUnwrapStmtStart() bool {
	if !p.check(TokenIdentifier) || p.peek().Lexeme != "unwrap" {
		return false
	}
	if p.pos+1 >= len(p.tokens) {
		return false
	}
	next := p.tokens[p.pos+1]
	if next.Type == TokenLBrace || next.Type == TokenIdentifier || next.Type == TokenUnder {
		return true
	}
	return false
}

func (p *Parser) parseUnwrapBlockStmt(start Token) (Statement, error) {
	open, err := p.consume(TokenLBrace, "expected '{' after 'unwrap'")
	if err != nil {
		return nil, err
	}
	p.unwrapBlockDepth++
	defer func() { p.unwrapBlockDepth-- }()
	var clauses []*UnwrapStmt
	var declared []string
	for !p.check(TokenRBrace) && !p.isAtEnd() {
		stmt, err := p.parseStatement()
		if err != nil {
			return nil, err
		}
		clause, ok := stmt.(*UnwrapStmt)
		if !ok {
			return nil, fmt.Errorf("unwrap body supports only '<-' unwrap bindings, got %T", stmt)
		}
		clauses = append(clauses, clause)
		for _, binding := range clause.Bindings {
			if binding.Name != "_" {
				declared = append(declared, binding.Name)
			}
		}
	}
	close, err := p.consume(TokenRBrace, "expected '}' after unwrap body")
	if err != nil {
		return nil, err
	}
	for _, name := range declared {
		p.declare(name)
	}
	if p.match(TokenElse) {
		elseToken := p.previous()
		fallback, err := p.parseBlockOrInlineStmtBody(elseToken, "unwrap else")
		if err != nil {
			return nil, err
		}
		span := mergeSpans(tokenSpan(start), fallback.Span)
		span = mergeSpans(span, mergeSpans(tokenSpan(open), tokenSpan(close)))
		return &GuardBlockStmt{Clauses: clauses, Fallback: fallback, Span: span}, nil
	}
	span := mergeSpans(tokenSpan(start), mergeSpans(tokenSpan(open), tokenSpan(close)))
	return &UnwrapBlockStmt{Clauses: clauses, Span: span}, nil
}

func (p *Parser) parseBindingsWithStart(start Token, firstIsName bool) ([]Binding, error) {
	var bindings []Binding
	useStart := start
	first := true
	for {
		binding := Binding{}

		name := useStart
		if !first || !firstIsName {
			var err error
			if p.check(TokenIdentifier) {
				name, err = p.consume(TokenIdentifier, "expected binding name")
			} else {
				name, err = p.consume(TokenUnder, "expected binding name")
			}
			if err != nil {
				return nil, err
			}
		}
		binding.Name = name.Lexeme
		binding.Span = tokenSpan(name)
		if p.bindingTypeStartsOnSameLine(name) && (p.check(TokenIdentifier) || p.check(TokenLParen) || p.check(TokenLBrace)) {
			typeRef, err := p.parseTypeRef()
			if err != nil {
				return nil, err
			}
			binding.Type = typeRef
			binding.Span = mergeSpans(binding.Span, typeSpan(typeRef))
		}
		bindings = append(bindings, binding)
		if !p.match(TokenComma) {
			break
		}
		first = false
		useStart = Token{}
	}
	return bindings, nil
}

func (p *Parser) isBareBindingStart() bool {
	if p.check(TokenVar) {
		return true
	}
	if !p.check(TokenIdentifier) && !p.check(TokenUnder) {
		return false
	}

	if p.checkNext(TokenAssign) {
		return true
	}

	if p.checkNext(TokenLeftArrow) {
		return true
	}

	if p.checkNext(TokenIdentifier) || p.checkNext(TokenLParen) || p.checkNext(TokenLBrace) || p.checkNext(TokenComma) {
		return p.bindingListFollowedByAssign(p.pos)
	}

	return false
}

func (p *Parser) bindingListFollowedByAssign(start int) bool {
	i := start
	if i < len(p.tokens) && p.tokens[i].Type == TokenVar {
		i++
	}
	for {
		if i >= len(p.tokens) || (p.tokens[i].Type != TokenIdentifier && p.tokens[i].Type != TokenUnder) {
			return false
		}
		i++
		if i < len(p.tokens) && p.sameLineTokens(i-1, i) && (p.tokens[i].Type == TokenIdentifier || p.tokens[i].Type == TokenLParen || p.tokens[i].Type == TokenLBrace) {
			end, ok := p.scanTypeRef(i)
			if !ok {
				return false
			}
			i = end
		}
		if i >= len(p.tokens) {
			return false
		}
		if p.tokens[i].Type == TokenAssign || p.tokens[i].Type == TokenLeftArrow {
			return true
		}
		if p.tokens[i].Type != TokenComma {
			return false
		}
		i++
	}
}

func (p *Parser) parseBindingInitializers(count int, operator Token) ([]Expr, error) {
	if count <= 0 {
		return nil, nil
	}
	if err := p.requireSameLineExpressionStart(operator); err != nil {
		return nil, err
	}
	if p.match(TokenQuestion) {
		values := []Expr{nil}
		for len(values) < count && p.match(TokenComma) {
			if !p.match(TokenQuestion) {
				return nil, fmt.Errorf("expected '?' after ',' in initializer list, got %s", p.peek().String())
			}
			values = append(values, nil)
		}
		return values, nil
	}
	first, err := p.parseExpression(0)
	if err != nil {
		return nil, err
	}
	values := []Expr{first}
	if count > 1 && !p.check(TokenComma) {
		return values, nil
	}
	for len(values) < count && p.match(TokenComma) {
		if p.match(TokenQuestion) {
			values = append(values, nil)
			continue
		}
		expr, err := p.parseExpression(0)
		if err != nil {
			return nil, err
		}
		values = append(values, expr)
	}
	return values, nil
}

func (p *Parser) parseExprList(until TokenType) ([]Expr, error) {
	var values []Expr
	for {
		expr, err := p.parseExpression(0)
		if err != nil {
			return nil, err
		}
		values = append(values, expr)
		if !p.match(TokenComma) {
			break
		}
		if p.check(until) {
			break
		}
	}
	return values, nil
}

func (p *Parser) parseIfStmt() (Statement, error) {
	start := p.advance()
	return p.parseIfStmtAfterStart(start)
}

func (p *Parser) parseIfStmtAfterStart(start Token) (Statement, error) {
	stmt := &IfStmt{}
	if p.match(TokenLet) {
		if p.check(TokenLBrace) {
			clauses, _, err := p.parseRefutableClauseBlock("if let")
			if err != nil {
				return nil, err
			}
			stmt.PatternClauses = clauses
		} else {
			pattern, err := p.parsePattern()
			if err != nil {
				return nil, err
			}
			if _, err := p.consume(TokenAssign, "expected '=' after if pattern"); err != nil {
				return nil, err
			}
			if err := p.requireSameLineExpressionStart(p.previous()); err != nil {
				return nil, err
			}
			value, err := p.parseExpressionUntil(TokenLBrace, TokenThen)
			if err != nil {
				return nil, err
			}
			stmt.Pattern = pattern
			stmt.PatternValue = value
		}
	} else {
		save := p.pos
		if pattern, err := p.parsePattern(); err == nil && p.check(TokenAssign) {
			_ = pattern
			return nil, fmt.Errorf("pattern matches in 'if' require 'let'; use 'if let Pattern = value { ... }'")
		}
		p.pos = save
		condition, err := p.parseExpressionUntil(TokenLBrace, TokenThen)
		if err != nil {
			return nil, err
		}
		stmt.Condition = condition
	}
	thenBlock, err := p.parseThenStmtBodyBlock("if", TokenElse)
	if err != nil {
		return nil, err
	}
	stmt.Then = thenBlock
	if p.match(TokenElse) {
		elseToken := p.previous()
		if p.check(TokenIf) {
			elseIfStmt, err := p.parseIfStmt()
			if err != nil {
				return nil, err
			}
			stmt.ElseIf = elseIfStmt.(*IfStmt)
			stmt.Span = mergeSpans(tokenSpan(start), stmt.ElseIf.Span)
			return stmt, nil
		}
		elseBlock, err := p.parseBlockOrInlineStmtBody(elseToken, "else")
		if err != nil {
			return nil, err
		}
		stmt.Else = elseBlock
		stmt.Span = mergeSpans(tokenSpan(start), elseBlock.Span)
		return stmt, nil
	}
	stmt.Span = mergeSpans(tokenSpan(start), thenBlock.Span)
	return stmt, nil
}

func (p *Parser) parseMatchStmt() (Statement, error) {
	start, err := p.consume(TokenMatch, "expected 'match'")
	if err != nil {
		return nil, err
	}
	return p.parseMatchStmtAfterStart(start, false)
}

func (p *Parser) parsePartialStmt() (Statement, error) {
	start, err := p.consume(TokenPartial, "expected 'partial'")
	if err != nil {
		return nil, err
	}
	return p.parseMatchStmtAfterStart(start, true)
}

func (p *Parser) parseMatchStmtAfterStart(start Token, partial bool) (Statement, error) {
	value, err := p.parseExpressionUntil(TokenLBrace)
	if err != nil {
		return nil, err
	}
	if !p.check(TokenLBrace) {
		return nil, fmt.Errorf("match requires a block of cases")
	}
	var cases []MatchCase
	var end Token
	cases, end, err = p.parseMatchCases()
	if err != nil {
		return nil, err
	}
	stmt := &MatchStmt{Partial: partial, Value: value, Cases: cases}
	stmt.Span = mergeSpans(tokenSpan(start), tokenSpan(end))
	return stmt, nil
}

func (p *Parser) parseMatchCases() ([]MatchCase, Token, error) {
	if _, err := p.consume(TokenLBrace, "expected '{' after match value"); err != nil {
		return nil, Token{}, err
	}
	var cases []MatchCase
	for !p.check(TokenRBrace) && !p.isAtEnd() {
		if _, err := p.consume(TokenCase, "expected 'case' before match pattern"); err != nil {
			return nil, Token{}, err
		}
		pattern, err := p.parsePattern()
		if err != nil {
			return nil, Token{}, err
		}
		matchCase := MatchCase{Pattern: pattern}
		if matchCase.Guard, err = p.parseOptionalMatchGuard(); err != nil {
			return nil, Token{}, err
		}
		if _, err := p.consume(TokenFatArrow, "expected '=>' after match pattern"); err != nil {
			return nil, Token{}, err
		}
		if p.check(TokenLBrace) {
			body, err := p.parseBlock()
			if err != nil {
				return nil, Token{}, err
			}
			matchCase.Body = body
			matchCase.Span = mergeSpans(patternSpan(pattern), body.Span)
		} else {
			expr, err := p.parseExpressionWithOptions(0, false)
			if err != nil {
				return nil, Token{}, err
			}
			matchCase.Expr = expr
			matchCase.Span = mergeSpans(patternSpan(pattern), exprSpan(expr))
		}
		cases = append(cases, matchCase)
	}
	end, err := p.consume(TokenRBrace, "expected '}' after match cases")
	if err != nil {
		return nil, Token{}, err
	}
	return cases, end, nil
}

func (p *Parser) parseOptionalMatchGuard() (Expr, error) {
	if !p.match(TokenIf) {
		return nil, nil
	}
	if err := p.requireSameLineExpressionStart(p.previous()); err != nil {
		return nil, err
	}
	guard, err := p.parseExpressionUntil(TokenFatArrow)
	if err != nil {
		return nil, err
	}
	return guard, nil
}

func (p *Parser) parsePattern() (Pattern, error) {
	return p.parsePatternAtDepth(0)
}

func (p *Parser) parsePatternAtDepth(depth int) (Pattern, error) {
	token := p.advance()
	switch token.Type {
	case TokenUnder:
		if p.bindingTypeStartsOnSameLine(token) && (p.check(TokenIdentifier) || p.check(TokenLBrace)) {
			if depth > 0 {
				return nil, fmt.Errorf("nested type patterns are not supported")
			}
			target, err := p.parseTypeRef()
			if err != nil {
				return nil, err
			}
			return &TypePattern{Name: "_", Target: target, Span: mergeSpans(tokenSpan(token), typeSpan(target))}, nil
		}
		return &WildcardPattern{Span: tokenSpan(token)}, nil
	case TokenInteger:
		return &LiteralPattern{Value: &IntegerLiteral{Value: token.Lexeme, Span: tokenSpan(token)}, Span: tokenSpan(token)}, nil
	case TokenFloat:
		return &LiteralPattern{Value: &FloatLiteral{Value: token.Lexeme, Span: tokenSpan(token)}, Span: tokenSpan(token)}, nil
	case TokenRune:
		return &LiteralPattern{Value: &RuneLiteral{Value: token.Lexeme, Span: tokenSpan(token)}, Span: tokenSpan(token)}, nil
	case TokenBool:
		return &LiteralPattern{Value: &BoolLiteral{Value: token.Lexeme == "true", Span: tokenSpan(token)}, Span: tokenSpan(token)}, nil
	case TokenString:
		return &LiteralPattern{Value: &StringLiteral{Value: token.Lexeme, Span: tokenSpan(token)}, Span: tokenSpan(token)}, nil
	case TokenMultilineString:
		return &LiteralPattern{Value: &StringLiteral{Value: token.Lexeme, Span: tokenSpan(token)}, Span: tokenSpan(token)}, nil
	case TokenLParen:
		if depth > 1 {
			return nil, fmt.Errorf("match patterns support at most 2 levels of nesting")
		}
		if p.check(TokenRParen) {
			end, err := p.consume(TokenRParen, "expected ')'")
			if err != nil {
				return nil, err
			}
			span := mergeSpans(tokenSpan(token), tokenSpan(end))
			return &LiteralPattern{Value: &UnitLiteral{Span: span}, Span: span}, nil
		}
		first, err := p.parsePatternAtDepth(depth + 1)
		if err != nil {
			return nil, err
		}
		if !p.match(TokenComma) {
			if _, err := p.consume(TokenRParen, "expected ')' after pattern"); err != nil {
				return nil, err
			}
			return first, nil
		}
		elements := []Pattern{first}
		for {
			next, err := p.parsePatternAtDepth(depth + 1)
			if err != nil {
				return nil, err
			}
			elements = append(elements, next)
			if !p.match(TokenComma) {
				break
			}
		}
		end, err := p.consume(TokenRParen, "expected ')' after tuple pattern")
		if err != nil {
			return nil, err
		}
		return &TuplePattern{Elements: elements, Span: mergeSpans(tokenSpan(token), tokenSpan(end))}, nil
	case TokenIdentifier:
		if p.bindingTypeStartsOnSameLine(token) && (p.check(TokenIdentifier) || p.check(TokenLBrace)) {
			if depth > 0 {
				return nil, fmt.Errorf("nested type patterns are not supported")
			}
			target, err := p.parseTypeRef()
			if err != nil {
				return nil, err
			}
			return &TypePattern{Name: token.Lexeme, Target: target, Span: mergeSpans(tokenSpan(token), typeSpan(target))}, nil
		}
		path := []string{token.Lexeme}
		endSpan := tokenSpan(token)
		for p.match(TokenDot) {
			next, err := p.consume(TokenIdentifier, "expected identifier after '.'")
			if err != nil {
				return nil, err
			}
			path = append(path, next.Lexeme)
			endSpan = tokenSpan(next)
		}
		if p.match(TokenLParen) {
			if depth > 1 {
				return nil, fmt.Errorf("match patterns support at most 2 levels of nesting")
			}
			var args []Pattern
			if !p.check(TokenRParen) {
				for {
					arg, err := p.parsePatternAtDepth(depth + 1)
					if err != nil {
						return nil, err
					}
					args = append(args, arg)
					if !p.match(TokenComma) {
						break
					}
				}
			}
			end, err := p.consume(TokenRParen, "expected ')' after constructor pattern")
			if err != nil {
				return nil, err
			}
			return &ConstructorPattern{Path: path, Args: args, Span: mergeSpans(tokenSpan(token), tokenSpan(end))}, nil
		}
		if len(path) == 1 {
			return &BindingPattern{Name: path[0], Span: tokenSpan(token)}, nil
		}
		return &ConstructorPattern{Path: path, Span: mergeSpans(tokenSpan(token), endSpan)}, nil
	default:
		return nil, fmt.Errorf("unexpected token in pattern %s", token.String())
	}
}

func (p *Parser) bindingListFollowedByArrow(start int) bool {
	if start >= len(p.tokens) || (p.tokens[start].Type != TokenIdentifier && p.tokens[start].Type != TokenUnder) {
		return false
	}

	i := start + 1
	for {
		if i < len(p.tokens) && p.sameLineTokens(i-1, i) && (p.tokens[i].Type == TokenIdentifier || p.tokens[i].Type == TokenLParen || p.tokens[i].Type == TokenLBrace) {
			end, ok := p.scanTypeRef(i)
			if !ok {
				return false
			}
			i = end
		}
		if i < len(p.tokens) && p.tokens[i].Type == TokenLeftArrow {
			return true
		}
		if i >= len(p.tokens) || p.tokens[i].Type != TokenComma {
			return false
		}
		i++
		if i >= len(p.tokens) || (p.tokens[i].Type != TokenIdentifier && p.tokens[i].Type != TokenUnder) {
			return false
		}
		i++
	}
}

func (p *Parser) bindingTypeStartsOnSameLine(name Token) bool {
	if p.pos >= len(p.tokens) {
		return false
	}
	return sameLine(name, p.tokens[p.pos])
}

func (p *Parser) sameLineTokens(leftIndex, rightIndex int) bool {
	if leftIndex < 0 || rightIndex < 0 || leftIndex >= len(p.tokens) || rightIndex >= len(p.tokens) {
		return false
	}
	return sameLine(p.tokens[leftIndex], p.tokens[rightIndex])
}

func sameLine(left Token, right Token) bool {
	return left.EndLine == right.Line
}

func (p *Parser) parseForStmt() (Statement, error) {
	start := p.advance()
	return p.parseForStmtAfterStart(start)
}

func (p *Parser) parseWhileStmt() (Statement, error) {
	start, err := p.consume(TokenWhile, "expected 'while'")
	if err != nil {
		return nil, err
	}
	condition, err := p.parseExpressionUntil(TokenLBrace)
	if err != nil {
		return nil, err
	}
	body, err := p.parseRequiredBlock("while")
	if err != nil {
		return nil, err
	}
	return &WhileStmt{
		Condition: condition,
		Body:      body,
		Span:      mergeSpans(tokenSpan(start), body.Span),
	}, nil
}

func (p *Parser) parseForStmtAfterStart(start Token) (Statement, error) {
	p.beginScope()
	defer p.endScope()
	if ((p.check(TokenIdentifier) || p.check(TokenUnder)) && p.bindingListFollowedByArrow(p.pos)) ||
		(p.check(TokenVar) && p.bindingListFollowedByAssign(p.pos)) {
		binding, err := p.parseForClause()
		if err != nil {
			return nil, err
		}
		p.declareBindings(binding.Bindings)
		if p.check(TokenYield) {
			if _, err := p.consume(TokenYield, "expected 'yield' after for binding"); err != nil {
				return nil, err
			}
			yieldBody, err := p.parseYieldBodyBlock("yield")
			if err != nil {
				return nil, err
			}
			return &ForStmt{
				Bindings:  []ForBinding{binding},
				YieldBody: yieldBody,
				Span:      mergeSpans(tokenSpan(start), yieldBody.Span),
			}, nil
		}
		if !p.check(TokenLBrace) && !p.isAtEnd() && sameLine(start, p.peek()) {
			return nil, fmt.Errorf("for requires a '{ ... }' block body; one-line for forms are not supported")
		}
		body, err := p.parseRequiredBlock("for")
		if err != nil {
			return nil, err
		}
		return &ForStmt{
			Bindings: []ForBinding{binding},
			Body:     body,
			Span:     mergeSpans(tokenSpan(start), body.Span),
		}, nil
	}
	if p.check(TokenLBrace) && p.isForYieldStart() {
		bindings, err := p.parseForBindingsBlock()
		if err != nil {
			return nil, err
		}
		if _, err := p.consume(TokenYield, "expected 'yield' after for bindings"); err != nil {
			return nil, err
		}
		yieldBody, err := p.parseYieldBodyBlock("yield")
		if err != nil {
			return nil, err
		}
		return &ForStmt{
			Bindings:  bindings,
			YieldBody: yieldBody,
			Span:      mergeSpans(tokenSpan(start), yieldBody.Span),
		}, nil
	}
	return nil, fmt.Errorf("for requires '<-' bindings or 'yield'; use 'while' for condition loops")
}

func (p *Parser) parseRequiredBlock(owner string) (*BlockStmt, error) {
	if !p.check(TokenLBrace) {
		if owner == "for" {
			return nil, fmt.Errorf("for requires a '{ ... }' block body; one-line for forms are not supported")
		}
		return nil, fmt.Errorf("expected '{' after %s", owner)
	}
	return p.parseBlock()
}

func (p *Parser) parseThenStmtBodyBlock(owner string, stopTypes ...TokenType) (*BlockStmt, error) {
	if p.check(TokenLBrace) {
		return p.parseBlock()
	}
	then, err := p.consume(TokenThen, "expected '{' or 'then' after "+owner)
	if err != nil {
		return nil, err
	}
	if p.isAtEnd() {
		return nil, fmt.Errorf("expected statement after 'then'")
	}
	if !sameLine(then, p.peek()) {
		return nil, fmt.Errorf("%s then-body must stay on the same line unless it uses '{ ... }'", owner)
	}
	p.beginScope()
	defer p.endScope()
	stmt, err := p.parseInlineStatement(stopTypes...)
	if err != nil {
		return nil, err
	}
	return &BlockStmt{
		Statements: []Statement{stmt},
		Span:       mergeSpans(tokenSpan(then), stmtSpan(stmt)),
	}, nil
}

func (p *Parser) parseYieldBodyBlock(owner string, stopTypes ...TokenType) (*BlockStmt, error) {
	introducer := p.previous()
	if p.check(TokenLBrace) {
		return p.parseBlock()
	}
	if p.isAtEnd() {
		return nil, fmt.Errorf("expected '{' or expression after %s", owner)
	}
	if !sameLine(introducer, p.peek()) {
		return nil, fmt.Errorf("%s body must stay on the same line unless it uses '{ ... }'", owner)
	}
	var (
		expr Expr
		err  error
	)
	if len(stopTypes) > 0 {
		expr, err = p.parseInlineExpression(stopTypes...)
	} else {
		expr, err = p.parseExpression(0)
	}
	if err != nil {
		return nil, err
	}
	stmt := &ExprStmt{Expr: expr, Span: exprSpan(expr)}
	return &BlockStmt{
		Statements: []Statement{stmt},
		Span:       mergeSpans(tokenSpan(introducer), exprSpan(expr)),
	}, nil
}

func (p *Parser) parseInlineMatchCases(statementMode bool) ([]MatchCase, Token, error) {
	if _, err := p.consume(TokenColon, "expected '{' or ':' after match value"); err != nil {
		return nil, Token{}, err
	}
	if p.isAtEnd() {
		return nil, Token{}, fmt.Errorf("expected match case after ':'")
	}
	if _, err := p.consume(TokenCase, "expected 'case' before match pattern"); err != nil {
		return nil, Token{}, err
	}
	pattern, err := p.parsePattern()
	if err != nil {
		return nil, Token{}, err
	}
	matchCase := MatchCase{Pattern: pattern}
	if matchCase.Guard, err = p.parseOptionalMatchGuard(); err != nil {
		return nil, Token{}, err
	}
	arrow, err := p.consume(TokenFatArrow, "expected '=>' after match pattern")
	if err != nil {
		return nil, Token{}, err
	}
	if p.isAtEnd() {
		return nil, Token{}, fmt.Errorf("expected match case body after '=>'")
	}
	if statementMode {
		p.beginScope()
		var stmt Statement
		if sameLine(arrow, p.peek()) {
			stmt, err = p.parseInlineStatement()
		} else {
			stmt, err = p.parseStatement()
		}
		p.endScope()
		if err != nil {
			return nil, Token{}, err
		}
		matchCase.Body = &BlockStmt{
			Statements: []Statement{stmt},
			Span:       stmtSpan(stmt),
		}
		matchCase.Span = mergeSpans(patternSpan(pattern), stmtSpan(stmt))
	} else {
		var expr Expr
		if sameLine(arrow, p.peek()) {
			expr, err = p.parseInlineExpression()
		} else {
			expr, err = p.parseExpression(0)
		}
		if err != nil {
			return nil, Token{}, err
		}
		matchCase.Expr = expr
		matchCase.Span = mergeSpans(patternSpan(pattern), exprSpan(expr))
	}
	return []MatchCase{matchCase}, p.previous(), nil
}

func (p *Parser) parseBlockOrInlineStmtBody(introducer Token, owner string, stopTypes ...TokenType) (*BlockStmt, error) {
	if p.check(TokenLBrace) {
		return p.parseBlock()
	}
	if p.isAtEnd() {
		return nil, fmt.Errorf("expected '{' or inline statement after %s", owner)
	}
	if !sameLine(introducer, p.peek()) {
		return nil, fmt.Errorf("%s body must stay on the same line unless it uses '{ ... }'", owner)
	}
	p.beginScope()
	defer p.endScope()
	stmt, err := p.parseInlineStatement(stopTypes...)
	if err != nil {
		return nil, err
	}
	return &BlockStmt{
		Statements: []Statement{stmt},
		Span:       mergeSpans(tokenSpan(introducer), stmtSpan(stmt)),
	}, nil
}

func (p *Parser) parseInlineStatement(stopTypes ...TokenType) (Statement, error) {
	sub, nextPos, err := p.inlineBodyParser(stopTypes...)
	if err != nil {
		return nil, err
	}
	stmt, err := sub.parseStatement()
	if err != nil {
		return nil, err
	}
	if !sub.isAtEnd() {
		return nil, fmt.Errorf("expected end of inline statement, got %s", sub.peek().String())
	}
	p.pos = nextPos
	return stmt, nil
}

func (p *Parser) parseInlineExpression(stopTypes ...TokenType) (Expr, error) {
	sub, nextPos, err := p.inlineBodyParser(stopTypes...)
	if err != nil {
		return nil, err
	}
	expr, err := sub.parseExpressionWithOptions(0, false)
	if err != nil {
		return nil, err
	}
	if !sub.isAtEnd() {
		return nil, fmt.Errorf("expected end of inline expression, got %s", sub.peek().String())
	}
	p.pos = nextPos
	return expr, nil
}

func (p *Parser) parseExpressionUntil(stopTypes ...TokenType) (Expr, error) {
	sub, nextPos, err := p.subparserUntil(stopTypes...)
	if err != nil {
		return nil, err
	}
	expr, err := sub.parseExpressionWithOptions(0, false)
	if err != nil {
		return nil, err
	}
	if !sub.isAtEnd() {
		return nil, fmt.Errorf("expected end of expression, got %s", sub.peek().String())
	}
	p.pos = nextPos
	return expr, nil
}

func (p *Parser) inlineBodyParser(stopTypes ...TokenType) (*Parser, int, error) {
	if p.isAtEnd() {
		return nil, 0, fmt.Errorf("expected inline body")
	}
	line := p.peek().Line
	stopSet := map[TokenType]struct{}{}
	for _, stop := range stopTypes {
		stopSet[stop] = struct{}{}
	}
	depthParen := 0
	depthBracket := 0
	depthBrace := 0
	end := p.pos
	for end < len(p.tokens) {
		token := p.tokens[end]
		if token.Line != line && depthParen == 0 && depthBracket == 0 && depthBrace == 0 {
			break
		}
		if depthParen == 0 && depthBracket == 0 && depthBrace == 0 {
			if _, ok := stopSet[token.Type]; ok {
				break
			}
		}
		switch token.Type {
		case TokenLParen:
			depthParen++
		case TokenRParen:
			if depthParen > 0 {
				depthParen--
			}
		case TokenLBracket:
			depthBracket++
		case TokenRBracket:
			if depthBracket > 0 {
				depthBracket--
			}
		case TokenLBrace:
			depthBrace++
		case TokenRBrace:
			if depthBrace > 0 {
				depthBrace--
			}
		}
		end++
	}
	if end == p.pos {
		return nil, 0, fmt.Errorf("expected inline body")
	}
	inlineTokens := append([]Token(nil), p.tokens[p.pos:end]...)
	last := inlineTokens[len(inlineTokens)-1]
	inlineTokens = append(inlineTokens, Token{
		Type:      TokenEOF,
		Line:      last.EndLine,
		Column:    last.EndColumn,
		EndLine:   last.EndLine,
		EndColumn: last.EndColumn,
	})
	scopes := make([]map[string]struct{}, len(p.scopes))
	copy(scopes, p.scopes)
	return &Parser{tokens: inlineTokens, scopes: scopes, multilineExprDepth: p.multilineExprDepth, unwrapBlockDepth: p.unwrapBlockDepth}, end, nil
}

func (p *Parser) subparserUntil(stopTypes ...TokenType) (*Parser, int, error) {
	if p.isAtEnd() {
		return nil, 0, fmt.Errorf("expected expression")
	}
	stopSet := map[TokenType]struct{}{}
	for _, stop := range stopTypes {
		stopSet[stop] = struct{}{}
	}
	depthParen := 0
	depthBracket := 0
	depthBrace := 0
	end := p.pos
	for end < len(p.tokens) {
		token := p.tokens[end]
		if depthParen == 0 && depthBracket == 0 && depthBrace == 0 {
			if _, ok := stopSet[token.Type]; ok {
				break
			}
		}
		switch token.Type {
		case TokenLParen:
			depthParen++
		case TokenRParen:
			if depthParen > 0 {
				depthParen--
			}
		case TokenLBracket:
			depthBracket++
		case TokenRBracket:
			if depthBracket > 0 {
				depthBracket--
			}
		case TokenLBrace:
			depthBrace++
		case TokenRBrace:
			if depthBrace > 0 {
				depthBrace--
			}
		}
		end++
	}
	if end == p.pos {
		return nil, 0, fmt.Errorf("expected expression")
	}
	inlineTokens := append([]Token(nil), p.tokens[p.pos:end]...)
	last := inlineTokens[len(inlineTokens)-1]
	inlineTokens = append(inlineTokens, Token{
		Type:      TokenEOF,
		Line:      last.EndLine,
		Column:    last.EndColumn,
		EndLine:   last.EndLine,
		EndColumn: last.EndColumn,
	})
	scopes := make([]map[string]struct{}, len(p.scopes))
	copy(scopes, p.scopes)
	return &Parser{tokens: inlineTokens, scopes: scopes, multilineExprDepth: p.multilineExprDepth, unwrapBlockDepth: p.unwrapBlockDepth}, end, nil
}

func (p *Parser) parseForClause() (ForBinding, error) {
	mutableDecl := false
	if p.match(TokenVar) {
		mutableDecl = true
	}
	var name Token
	var err error
	if p.check(TokenIdentifier) {
		name, err = p.consume(TokenIdentifier, "expected loop variable")
	} else {
		name, err = p.consume(TokenUnder, "expected loop variable")
	}
	if err != nil {
		return ForBinding{}, err
	}
	bindings, err := p.parseBindingsWithStart(name, true)
	if err != nil {
		return ForBinding{}, err
	}
	switch p.peek().Type {
	case TokenLeftArrow:
		if mutableDecl {
			return ForBinding{}, fmt.Errorf("for generators cannot use 'var'; use 'var name = ...' only for local mutable bindings")
		}
		p.advance()
		if err := p.requireSameLineExpressionStart(p.previous()); err != nil {
			return ForBinding{}, err
		}
		iterable, err := p.parseInlineExpression(TokenComma, TokenRBrace, TokenYield, TokenLBrace, TokenColon)
		if err != nil {
			if isForInlineBodyParseError(err) {
				return ForBinding{}, fmt.Errorf("for requires a '{ ... }' block body; one-line for forms are not supported")
			}
			return ForBinding{}, err
		}
		return ForBinding{
			Bindings: bindings,
			Iterable: iterable,
			Span:     mergeSpans(tokenSpan(name), exprSpan(iterable)),
		}, nil
	case TokenAssign, TokenColonAssign:
		operator := p.advance()
		if operator.Type == TokenColonAssign {
			return ForBinding{}, fmt.Errorf("cannot use ':=' in a declaration; use 'var' with '=' for mutable bindings")
		}
		for i := range bindings {
			bindings[i].Mutable = mutableDecl
		}
		values, err := p.parseBindingInitializers(len(bindings), operator)
		if err != nil {
			return ForBinding{}, err
		}
		if mutableDecl {
			for i := range values {
				if values[i] == nil {
					return ForBinding{}, fmt.Errorf("mutable declarations must initialize with '='; deferred '?' initializers are not supported")
				}
			}
		}
		for i := range bindings {
			if i < len(values) && values[i] != nil {
				values[i] = wrapThunkExpr(bindings[i].Type, values[i])
			}
		}
		end := tokenSpan(name)
		for i, value := range values {
			if value != nil {
				end = mergeSpans(end, exprSpan(value))
				continue
			}
			if i < len(bindings) {
				end = mergeSpans(end, bindings[i].Span)
			}
		}
		return ForBinding{
			Bindings: bindings,
			Values:   values,
			Span:     end,
		}, nil
	default:
		return ForBinding{}, fmt.Errorf("expected '<-' or '=' after for bindings, got %s", p.peek().String())
	}
}

func isForInlineBodyParseError(err error) bool {
	if err == nil {
		return false
	}
	msg := err.Error()
	return strings.HasPrefix(msg, "expected end of inline expression, got ") ||
		strings.HasPrefix(msg, "expected end of expression, got ")
}

func (p *Parser) parseForBindingsBlock() ([]ForBinding, error) {
	if _, err := p.consume(TokenLBrace, "expected '{' after 'for'"); err != nil {
		return nil, err
	}
	var bindings []ForBinding
	if !p.check(TokenRBrace) {
		for {
			binding, err := p.parseForClause()
			if err != nil {
				return nil, err
			}
			bindings = append(bindings, binding)
			p.declareBindings(binding.Bindings)
			if p.match(TokenComma) {
				continue
			}
			if (p.check(TokenVar) || p.check(TokenIdentifier) || p.check(TokenUnder)) &&
				(p.bindingListFollowedByArrow(p.pos) || p.bindingListFollowedByAssign(p.pos)) {
				continue
			}
			break
		}
	}
	if _, err := p.consume(TokenRBrace, "expected '}' after for bindings"); err != nil {
		return nil, err
	}
	return bindings, nil
}

func (p *Parser) declareBindings(bindings []Binding) {
	for _, binding := range bindings {
		if binding.Name != "_" {
			p.declare(binding.Name)
		}
	}
}

func (p *Parser) isForYieldStart() bool {
	if !p.check(TokenLBrace) {
		return false
	}
	depthBrace := 0
	for i := p.pos; i < len(p.tokens); i++ {
		switch p.tokens[i].Type {
		case TokenLBrace:
			depthBrace++
		case TokenRBrace:
			depthBrace--
			if depthBrace == 0 {
				return i+1 < len(p.tokens) && p.tokens[i+1].Type == TokenYield
			}
		}
	}
	return false
}

func (p *Parser) parseReturnStmt() (Statement, error) {
	start := p.advance()
	value, err := p.parseExpression(0)
	if err != nil {
		return nil, err
	}
	return &ReturnStmt{Value: value, Span: mergeSpans(tokenSpan(start), exprSpan(value))}, nil
}

func (p *Parser) parseExprStmt() (Statement, error) {
	target, err := p.parseExpression(0)
	if err != nil {
		return nil, err
	}
	if p.match(TokenComma) {
		targets := []Expr{target}
		for {
			nextTarget, err := p.parseExpression(0)
			if err != nil {
				return nil, err
			}
			targets = append(targets, nextTarget)
			if !p.match(TokenComma) {
				break
			}
		}
		if !isAssignmentOperator(p.peek().Type) {
			return nil, fmt.Errorf("expected assignment operator after assignment targets at %d:%d", p.peek().Line, p.peek().Column)
		}
		operator := p.advance()
		values, err := p.parseAssignmentValues(operator)
		if err != nil {
			return nil, err
		}
		return &MultiAssignmentStmt{
			Targets:  targets,
			Operator: operator.Lexeme,
			Values:   values,
			Span:     mergeSpans(exprSpan(targets[0]), exprSpan(values[len(values)-1])),
		}, nil
	}
	if isAssignmentOperator(p.peek().Type) {
		operator := p.advance()
		if err := p.requireSameLineExpressionStart(operator); err != nil {
			return nil, err
		}
		value, err := p.parseExpression(0)
		if err != nil {
			return nil, err
		}
		return &AssignmentStmt{
			Target:   target,
			Operator: operator.Lexeme,
			Value:    value,
			Span:     mergeSpans(exprSpan(target), exprSpan(value)),
		}, nil
	}
	return &ExprStmt{Expr: target, Span: exprSpan(target)}, nil
}

func (p *Parser) parseAssignmentValues(operator Token) ([]Expr, error) {
	values := []Expr{}
	if err := p.requireSameLineExpressionStart(operator); err != nil {
		return nil, err
	}
	for {
		value, err := p.parseExpression(0)
		if err != nil {
			return nil, err
		}
		values = append(values, value)
		if !p.match(TokenComma) {
			break
		}
	}
	return values, nil
}

func isAssignmentOperator(tt TokenType) bool {
	switch tt {
	case TokenAssign, TokenColonAssign, TokenPlusEq, TokenMinusEq, TokenStarEq, TokenSlashEq, TokenPercentEq:
		return true
	default:
		return false
	}
}
