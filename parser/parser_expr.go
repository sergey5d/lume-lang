package parser

import "fmt"

func ParseExpr(input string) (Expr, error) {
	tokens, err := Lex(input)
	if err != nil {
		return nil, err
	}
	parser := &Parser{tokens: tokens}
	expr, err := parser.parseExpressionWithOptions(0, true)
	if err != nil {
		return nil, err
	}
	if !parser.check(TokenEOF) {
		return nil, fmt.Errorf("unexpected token %s", parser.peek().String())
	}
	return expr, nil
}

func (p *Parser) parseExpression(minPrec int) (Expr, error) {
	return p.parseExpressionWithOptions(minPrec, true)
}

func (p *Parser) parseExpressionWithOptions(minPrec int, allowRecordUpdate bool) (Expr, error) {
	left, err := p.parsePrefix()
	if err != nil {
		return nil, err
	}

	for {
		if p.shouldStopExpressionAtLineBreak() {
			break
		}
		if allowRecordUpdate && p.match(TokenWith) {
			updates, end, err := p.parseRecordUpdateArgs()
			if err != nil {
				return nil, err
			}
			left = &RecordUpdateExpr{Receiver: left, Updates: updates, Span: mergeSpans(exprSpan(left), tokenSpan(end))}
			continue
		}
		if p.check(TokenLParen) {
			args, err := p.parseCallArgs()
			if err != nil {
				return nil, err
			}
			call := &CallExpr{Callee: left, Args: args}
			endSpan := exprSpan(left)
			if len(args) > 0 {
				endSpan = args[len(args)-1].Span
			}
			call.Span = mergeSpans(exprSpan(left), endSpan)
			left = call
			continue
		}
		if p.isBraceLambdaStart() {
			lambda, err := p.parseBraceLambdaExpr()
			if err != nil {
				return nil, err
			}
			left = p.appendTrailingCallArg(left, lambda)
			continue
		}
		if p.isBareAnonymousRecordCallArgStart() {
			record, err := p.parseAnonymousRecordExpr(p.advance())
			if err != nil {
				return nil, err
			}
			left = p.appendTrailingCallArg(left, record)
			continue
		}
		if p.match(TokenDot) {
			name, err := p.consume(TokenIdentifier, "expected member name after '.'")
			if err != nil {
				return nil, err
			}
			left = &MemberExpr{Receiver: left, Name: name.Lexeme, Span: mergeSpans(exprSpan(left), tokenSpan(name))}
			continue
		}
		if p.match(TokenLBracket) {
			index, err := p.parseExpressionWithOptions(0, true)
			if err != nil {
				return nil, err
			}
			end, err := p.consume(TokenRBracket, "expected ']' after index expression")
			if err != nil {
				return nil, err
			}
			left = &IndexExpr{Receiver: left, Index: index, Span: mergeSpans(exprSpan(left), tokenSpan(end))}
			continue
		}

		op := p.peek().Type
		if op == TokenIs {
			prec := precedence(op)
			if prec < minPrec {
				break
			}
			p.advance()
			target, err := p.parseTypeRef()
			if err != nil {
				return nil, err
			}
			left = &IsExpr{
				Left:   left,
				Target: target,
				Span:   mergeSpans(exprSpan(left), typeSpan(target)),
			}
			continue
		}

		prec := precedence(op)
		if prec < minPrec {
			break
		}

		token := p.advance()
		right, err := p.parseExpressionWithOptions(prec+1, allowRecordUpdate)
		if err != nil {
			return nil, err
		}
		left = &BinaryExpr{
			Left:     left,
			Operator: token.Lexeme,
			Right:    right,
			Span:     mergeSpans(exprSpan(left), exprSpan(right)),
		}
	}

	return left, nil
}

func (p *Parser) appendTrailingCallArg(left Expr, arg Expr) Expr {
	callArg := CallArg{Value: arg, Span: exprSpan(arg)}
	if call, ok := left.(*CallExpr); ok {
		call.Args = append(call.Args, callArg)
		call.Span = mergeSpans(exprSpan(call.Callee), exprSpan(arg))
		return call
	}
	return &CallExpr{
		Callee: left,
		Args:   []CallArg{callArg},
		Span:   mergeSpans(exprSpan(left), exprSpan(arg)),
	}
}

func (p *Parser) parsePrefix() (Expr, error) {
	if p.check(TokenIdentifier) && p.isAnonymousInterfaceExprStart() {
		return p.parseAnonymousInterfaceExpr()
	}
	if p.isLambdaIdentifierStart() {
		return p.parseLambdaIdentifier()
	}
	if p.check(TokenLParen) && p.isLambdaParenStart() {
		return p.parseLambdaParen()
	}

	token := p.advance()
	switch token.Type {
	case TokenBang, TokenMinus, TokenTilde:
		right, err := p.parseExpression(unaryPrecedence())
		if err != nil {
			return nil, err
		}
		return &UnaryExpr{Operator: token.Lexeme, Right: right, Span: mergeSpans(tokenSpan(token), exprSpan(right))}, nil
	case TokenIdentifier:
		return &Identifier{Name: token.Lexeme, Span: tokenSpan(token)}, nil
	case TokenInteger:
		return &IntegerLiteral{Value: token.Lexeme, Span: tokenSpan(token)}, nil
	case TokenFloat:
		return &FloatLiteral{Value: token.Lexeme, Span: tokenSpan(token)}, nil
	case TokenRune:
		return &RuneLiteral{Value: token.Lexeme, Span: tokenSpan(token)}, nil
	case TokenBool:
		return &BoolLiteral{Value: token.Lexeme == "true", Span: tokenSpan(token)}, nil
	case TokenString:
		return p.parseStringLiteralExpr(token)
	case TokenMultilineString:
		value, err := decodeStringEscapes(token.Lexeme)
		if err != nil {
			return nil, fmt.Errorf("invalid multiline string at %d:%d: %w", token.Line, token.Column, err)
		}
		return &StringLiteral{Value: value, Span: tokenSpan(token)}, nil
	case TokenUnder:
		return &PlaceholderExpr{Span: tokenSpan(token)}, nil
	case TokenLParen:
		return p.parseParenExpr(token)
	case TokenLBracket:
		if p.match(TokenRBracket) {
			return &ListLiteral{Span: mergeSpans(tokenSpan(token), tokenSpan(p.previous()))}, nil
		}
		var items []Expr
		p.multilineExprDepth++
		defer func() { p.multilineExprDepth-- }()
		for {
			expr, err := p.parseExpressionWithOptions(0, true)
			if err != nil {
				return nil, err
			}
			items = append(items, expr)
			if !p.match(TokenComma) {
				break
			}
		}
		if _, err := p.consume(TokenRBracket, "expected ']'"); err != nil {
			return nil, err
		}
		return &ListLiteral{Elements: items, Span: mergeSpans(tokenSpan(token), tokenSpan(p.previous()))}, nil
	case TokenLBrace:
		block, err := p.parseBlockAfterStart(token)
		if err != nil {
			return nil, err
		}
		return &BlockExpr{Body: block, Span: block.Span}, nil
	case TokenRecord:
		return p.parseAnonymousRecordExpr(token)
	case TokenIf:
		return p.parseIfExprAfterStart(token)
	case TokenPartial:
		return p.parsePartialExprAfterStart(token)
	case TokenMatch:
		return p.parseMatchExprAfterStart(token)
	case TokenFor:
		return p.parseForYieldExprAfterStart(token)
	default:
		return nil, fmt.Errorf("unexpected token %s", token.String())
	}
}

func (p *Parser) isAnonymousInterfaceExprStart() bool {
	start := p.pos
	defer func() { p.pos = start }()

	if _, err := p.parseTypeRef(); err != nil {
		return false
	}
	for p.match(TokenWith) {
		if _, err := p.parseTypeRef(); err != nil {
			return false
		}
	}
	if !p.check(TokenLBrace) {
		return false
	}
	if p.pos+2 < len(p.tokens) && p.tokens[p.pos+1].Type == TokenIdentifier && p.tokens[p.pos+2].Type == TokenAssign {
		return false
	}
	return true
}

func (p *Parser) parseAnonymousRecordExpr(start Token) (Expr, error) {
	if start.Type == TokenLBrace || p.check(TokenLBrace) {
		if start.Type != TokenLBrace {
			p.advance()
		}
		var fields []CallArg
		if p.check(TokenRBrace) {
			return nil, fmt.Errorf("anonymous record literal must declare at least one field at %d:%d", start.Line, start.Column)
		}
		for {
			name, err := p.consume(TokenIdentifier, "expected record field name")
			if err != nil {
				return nil, err
			}
			if _, err := p.consume(TokenAssign, "expected '=' after record field name"); err != nil {
				return nil, err
			}
			value, err := p.parseExpressionWithOptions(0, true)
			if err != nil {
				return nil, err
			}
			fields = append(fields, CallArg{
				Name:  name.Lexeme,
				Value: value,
				Span:  mergeSpans(tokenSpan(name), exprSpan(value)),
			})
			if p.match(TokenComma) {
				continue
			}
			if p.check(TokenRBrace) {
				break
			}
			if p.check(TokenIdentifier) && exprSpan(value).End.Line < p.peek().Line {
				continue
			}
			return nil, fmt.Errorf("expected ',' or newline between record fields, got %s", p.peek().String())
		}
		end, err := p.consume(TokenRBrace, "expected '}' after anonymous record literal")
		if err != nil {
			return nil, err
		}
		return &AnonymousRecordExpr{
			Fields: fields,
			Span:   mergeSpans(tokenSpan(start), tokenSpan(end)),
		}, nil
	}
	if _, err := p.consume(TokenLParen, "expected '{' or '(' after 'record'"); err != nil {
		return nil, err
	}
	var values []Expr
	if !p.check(TokenRParen) {
		for {
			value, err := p.parseExpressionUntil(TokenComma, TokenRParen)
			if err != nil {
				return nil, err
			}
			values = append(values, value)
			if !p.match(TokenComma) {
				break
			}
		}
	}
	end, err := p.consume(TokenRParen, "expected ')' after positional anonymous record literal")
	if err != nil {
		return nil, err
	}
	if len(values) == 0 {
		return nil, fmt.Errorf("anonymous record literal must declare at least one value at %d:%d", start.Line, start.Column)
	}
	return &AnonymousRecordExpr{
		Values: values,
		Span:   mergeSpans(tokenSpan(start), tokenSpan(end)),
	}, nil
}

func (p *Parser) parseAnonymousInterfaceExpr() (Expr, error) {
	first, err := p.parseTypeRef()
	if err != nil {
		return nil, err
	}
	interfaces := []*TypeRef{first}
	for p.match(TokenWith) {
		iface, err := p.parseTypeRef()
		if err != nil {
			return nil, err
		}
		interfaces = append(interfaces, iface)
	}
	if _, err := p.consume(TokenLBrace, "expected '{' after anonymous interface list"); err != nil {
		return nil, err
	}
	var methods []*MethodDecl
	for !p.check(TokenRBrace) && !p.isAtEnd() {
		annotations, err := p.parseAnnotations()
		if err != nil {
			return nil, err
		}
		private := p.match(TokenPrivate)
		switch p.peek().Type {
		case TokenDef, TokenImpl:
			method, err := p.parseMethodLike(annotations, private)
			if err != nil {
				return nil, err
			}
			methods = append(methods, method)
		case TokenOperator:
			return nil, fmt.Errorf("use symbolic 'def' declarations instead of the 'operator' keyword")
		default:
			return nil, fmt.Errorf("expected anonymous interface member, got %s", p.peek().String())
		}
	}
	end, err := p.consume(TokenRBrace, "expected '}' after anonymous interface body")
	if err != nil {
		return nil, err
	}
	return &AnonymousInterfaceExpr{
		Interfaces: interfaces,
		Methods:    methods,
		Span:       mergeSpans(typeSpan(first), tokenSpan(end)),
	}, nil
}

func (p *Parser) parseStringLiteralExpr(token Token) (Expr, error) {
	raw := token.Lexeme
	span := tokenSpan(token)
	if !stringHasInterpolation(raw) {
		value, err := decodeStringEscapes(unescapeInterpolatedString(raw))
		if err != nil {
			return nil, fmt.Errorf("invalid string literal at %d:%d: %w", token.Line, token.Column, err)
		}
		return &StringLiteral{Value: value, Span: span}, nil
	}

	parts, err := parseInterpolatedStringParts(raw)
	if err != nil {
		return nil, fmt.Errorf("invalid string interpolation at %d:%d: %w", token.Line, token.Column, err)
	}

	exprs := make([]Expr, 0, len(parts)*2+1)
	if len(parts) == 0 || !parts[0].isLiteral {
		exprs = append(exprs, &StringLiteral{Value: "", Span: span})
	}
	for _, part := range parts {
		if part.isLiteral {
			value, err := decodeStringEscapes(part.text)
			if err != nil {
				return nil, fmt.Errorf("invalid string literal at %d:%d: %w", token.Line, token.Column, err)
			}
			exprs = append(exprs, &StringLiteral{Value: value, Span: span})
			continue
		}
		expr, err := ParseExpr(part.text)
		if err != nil {
			return nil, err
		}
		exprs = append(exprs, expr)
	}
	if len(exprs) == 1 {
		return exprs[0], nil
	}
	left := exprs[0]
	for _, right := range exprs[1:] {
		left = &BinaryExpr{
			Left:     left,
			Operator: "+",
			Right:    right,
			Span:     span,
		}
	}
	return left, nil
}

type interpolatedStringPart struct {
	isLiteral bool
	text      string
}

func stringHasInterpolation(raw string) bool {
	runes := []rune(raw)
	for i := 0; i < len(runes); i++ {
		if runes[i] != '$' {
			continue
		}
		if i > 0 && runes[i-1] == '\\' {
			continue
		}
		return true
	}
	return false
}

func unescapeInterpolatedString(raw string) string {
	runes := []rune(raw)
	var out []rune
	for i := 0; i < len(runes); i++ {
		if runes[i] == '\\' && i+1 < len(runes) && runes[i+1] == '$' {
			out = append(out, '$')
			i++
			continue
		}
		out = append(out, runes[i])
	}
	return string(out)
}

func decodeStringEscapes(raw string) (string, error) {
	runes := []rune(raw)
	out := make([]rune, 0, len(runes))
	for i := 0; i < len(runes); i++ {
		if runes[i] != '\\' {
			out = append(out, runes[i])
			continue
		}
		if i+1 >= len(runes) {
			return "", fmt.Errorf("dangling escape")
		}
		i++
		switch runes[i] {
		case 'n':
			out = append(out, '\n')
		case 't':
			out = append(out, '\t')
		case 'r':
			out = append(out, '\r')
		case '\\':
			out = append(out, '\\')
		case '"':
			out = append(out, '"')
		case '$':
			out = append(out, '$')
		default:
			return "", fmt.Errorf("unsupported escape \\%c", runes[i])
		}
	}
	return string(out), nil
}

func parseInterpolatedStringParts(raw string) ([]interpolatedStringPart, error) {
	runes := []rune(raw)
	var (
		parts   []interpolatedStringPart
		literal []rune
	)
	flushLiteral := func() {
		if len(literal) == 0 {
			return
		}
		parts = append(parts, interpolatedStringPart{isLiteral: true, text: string(literal)})
		literal = nil
	}

	for i := 0; i < len(runes); i++ {
		switch runes[i] {
		case '\\':
			if i+1 < len(runes) && runes[i+1] == '$' {
				literal = append(literal, '$')
				i++
				continue
			}
			literal = append(literal, runes[i])
		case '$':
			if i+1 >= len(runes) {
				return nil, fmt.Errorf("dangling '$'")
			}
			flushLiteral()
			if runes[i+1] == '{' {
				start := i + 2
				end, err := findInterpolatedExprEnd(runes, start)
				if err != nil {
					return nil, err
				}
				expr := string(runes[start:end])
				if expr == "" {
					return nil, fmt.Errorf("empty interpolation")
				}
				parts = append(parts, interpolatedStringPart{text: expr})
				i = end
				continue
			}
			if !isAlpha(runes[i+1]) {
				return nil, fmt.Errorf("expected identifier or '{' after '$'")
			}
			start := i + 1
			end := start + 1
			for end < len(runes) && isAlphaNumeric(runes[end]) {
				end++
			}
			parts = append(parts, interpolatedStringPart{text: string(runes[start:end])})
			i = end - 1
		default:
			literal = append(literal, runes[i])
		}
	}
	flushLiteral()
	return parts, nil
}

func findInterpolatedExprEnd(runes []rune, start int) (int, error) {
	depth := 1
	for i := start; i < len(runes); i++ {
		switch runes[i] {
		case '{':
			depth++
		case '}':
			depth--
			if depth == 0 {
				return i, nil
			}
		case '"':
			j := i + 1
			for ; j < len(runes); j++ {
				if runes[j] == '\\' {
					j++
					continue
				}
				if runes[j] == '"' {
					break
				}
			}
			if j >= len(runes) {
				return 0, fmt.Errorf("unterminated string inside interpolation")
			}
			i = j
		case '\'':
			j := i + 1
			for ; j < len(runes); j++ {
				if runes[j] == '\\' {
					j++
					continue
				}
				if runes[j] == '\'' {
					break
				}
			}
			if j >= len(runes) {
				return 0, fmt.Errorf("unterminated rune inside interpolation")
			}
			i = j
		}
	}
	return 0, fmt.Errorf("unterminated '${...}'")
}

func (p *Parser) parseIfExprAfterStart(start Token) (Expr, error) {
	condition, err := p.parseExpressionUntil(TokenLBrace, TokenThen)
	if err != nil {
		return nil, err
	}
	thenBlock, err := p.parseThenExprBodyBlock("if", TokenElse)
	if err != nil {
		return nil, err
	}
	elseToken, err := p.consume(TokenElse, "expected 'else' in if expression")
	if err != nil {
		return nil, err
	}
	var elseBlock *BlockStmt
	if p.check(TokenIf) {
		nestedStart := p.advance()
		nestedExpr, err := p.parseIfExprAfterStart(nestedStart)
		if err != nil {
			return nil, err
		}
		stmt := &ExprStmt{Expr: nestedExpr, Span: exprSpan(nestedExpr)}
		elseBlock = &BlockStmt{
			Statements: []Statement{stmt},
			Span:       mergeSpans(tokenSpan(elseToken), exprSpan(nestedExpr)),
		}
	} else {
		elseBlock, err = p.parseBlockOrInlineExprBody(elseToken, "else")
		if err != nil {
			return nil, err
		}
	}
	return &IfExpr{
		Condition: condition,
		Then:      thenBlock,
		Else:      elseBlock,
		Span:      mergeSpans(tokenSpan(start), elseBlock.Span),
	}, nil
}

func (p *Parser) parseBlockOrInlineExprBody(introducer Token, owner string, stopTypes ...TokenType) (*BlockStmt, error) {
	if p.check(TokenLBrace) {
		return p.parseBlock()
	}
	if p.isAtEnd() {
		return nil, fmt.Errorf("expected '{' or inline expression after %s", owner)
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

func (p *Parser) parseThenExprBodyBlock(owner string, stopTypes ...TokenType) (*BlockStmt, error) {
	introducer := p.previous()
	if p.check(TokenLBrace) {
		return p.parseBlock()
	}
	then, err := p.consume(TokenThen, "expected '{' or 'then' after "+owner)
	if err != nil {
		return nil, err
	}
	if p.isAtEnd() {
		return nil, fmt.Errorf("expected expression after 'then'")
	}
	if !sameLine(then, p.peek()) {
		return nil, fmt.Errorf("%s then-body must stay on the same line unless it uses '{ ... }'", owner)
	}
	var expr Expr
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

func (p *Parser) parseMatchExprAfterStart(start Token) (Expr, error) {
	return p.parseMatchExprAfterKeyword(start, false)
}

func (p *Parser) parsePartialExprAfterStart(start Token) (Expr, error) {
	return p.parseMatchExprAfterKeyword(start, true)
}

func (p *Parser) parseMatchExprAfterKeyword(start Token, partial bool) (Expr, error) {
	var (
		value Expr
		err   error
	)
	if p.check(TokenLBrace) {
		value = &PlaceholderExpr{Span: tokenSpan(start)}
	} else {
		value, err = p.parseExpressionUntil(TokenLBrace)
		if err != nil {
			return nil, err
		}
	}
	if !p.check(TokenLBrace) {
		return nil, fmt.Errorf("match requires a block of cases")
	}
	var (
		cases []MatchCase
		end   Token
	)
	cases, end, err = p.parseMatchCases()
	if err != nil {
		return nil, err
	}
	return &MatchExpr{
		Partial: partial,
		Value:   value,
		Cases:   cases,
		Span:    mergeSpans(tokenSpan(start), tokenSpan(end)),
	}, nil
}

func (p *Parser) parseForYieldExprAfterStart(start Token) (Expr, error) {
	p.beginScope()
	defer p.endScope()
	if (p.check(TokenIdentifier) || p.check(TokenUnder)) && p.bindingListFollowedByArrow(p.pos) {
		binding, err := p.parseForClause()
		if err != nil {
			return nil, err
		}
		p.declareBindings(binding.Bindings)
		if _, err := p.consume(TokenYield, "expected 'yield' after for binding"); err != nil {
			return nil, err
		}
		yieldBody, err := p.parseYieldBodyBlock("yield")
		if err != nil {
			return nil, err
		}
		return &ForYieldExpr{
			Bindings:  []ForBinding{binding},
			YieldBody: yieldBody,
			Span:      mergeSpans(tokenSpan(start), yieldBody.Span),
		}, nil
	}
	if !p.check(TokenLBrace) || !p.isForYieldStart() {
		return nil, fmt.Errorf("for expression requires 'for item <- items yield { ... }' or 'for { ... } yield { ... }'")
	}
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
	return &ForYieldExpr{
		Bindings:  bindings,
		YieldBody: yieldBody,
		Span:      mergeSpans(tokenSpan(start), yieldBody.Span),
	}, nil
}

func (p *Parser) parseCallArgs() ([]CallArg, error) {
	if _, err := p.consume(TokenLParen, "expected '('"); err != nil {
		return nil, err
	}
	var args []CallArg
	p.multilineExprDepth++
	defer func() { p.multilineExprDepth-- }()
	seenNamed := false
	if !p.check(TokenRParen) {
		for {
			if p.check(TokenIdentifier) && p.checkNext(TokenAssign) {
				nameToken := p.advance()
				p.advance()
				value, err := p.parseExpressionWithOptions(0, true)
				if err != nil {
					return nil, err
				}
				args = append(args, CallArg{
					Name:  nameToken.Lexeme,
					Value: value,
					Span:  mergeSpans(tokenSpan(nameToken), exprSpan(value)),
				})
				seenNamed = true
			} else {
				if seenNamed {
					return nil, fmt.Errorf("positional arguments cannot follow named arguments")
				}
				if p.isBareAnonymousRecordCallArgStart() {
					return nil, fmt.Errorf("bare '{ ... }' record arguments are not allowed inside '(...)'; use 'Type { ... }' or 'Type(record { ... })'")
				}
				expr, err := p.parseExpressionWithOptions(0, true)
				if err != nil {
					return nil, err
				}
				args = append(args, CallArg{
					Value: expr,
					Span:  exprSpan(expr),
				})
			}
			if !p.match(TokenComma) {
				break
			}
		}
	}
	if _, err := p.consume(TokenRParen, "expected ')' after arguments"); err != nil {
		return nil, err
	}
	return args, nil
}

func (p *Parser) isBareAnonymousRecordCallArgStart() bool {
	if !p.check(TokenLBrace) {
		return false
	}
	if p.pos+2 >= len(p.tokens) {
		return false
	}
	return p.tokens[p.pos+1].Type == TokenIdentifier && p.tokens[p.pos+2].Type == TokenAssign
}

func (p *Parser) parseRecordUpdateArgs() ([]CallArg, Token, error) {
	if _, err := p.consume(TokenLBrace, "expected '{'"); err != nil {
		return nil, Token{}, err
	}
	var updates []CallArg
	p.multilineExprDepth++
	defer func() { p.multilineExprDepth-- }()
	for {
		nameToken, err := p.consume(TokenIdentifier, "expected record field name")
		if err != nil {
			return nil, Token{}, err
		}
		if _, err := p.consume(TokenAssign, "expected '=' after record field name"); err != nil {
			return nil, Token{}, err
		}
		value, err := p.parseExpressionWithOptions(0, true)
		if err != nil {
			return nil, Token{}, err
		}
		updates = append(updates, CallArg{
			Name:  nameToken.Lexeme,
			Value: value,
			Span:  mergeSpans(tokenSpan(nameToken), exprSpan(value)),
		})
		if !p.match(TokenComma) {
			break
		}
	}
	end, err := p.consume(TokenRBrace, "expected '}' after record update")
	if err != nil {
		return nil, Token{}, err
	}
	return updates, end, nil
}

func precedence(t TokenType) int {
	switch t {
	case TokenOrOr:
		return 1
	case TokenAndAnd:
		return 2
	case TokenPipe:
		return 3
	case TokenAmp:
		return 4
	case TokenEqEq, TokenBangEq, TokenIs:
		return 5
	case TokenLT, TokenLTE, TokenGT, TokenGTE:
		return 6
	case TokenLTLT, TokenGTGT:
		return 7
	case TokenPlus, TokenMinus, TokenPlusPlus, TokenMinusMinus, TokenColonPlus, TokenColonMinus:
		return 8
	case TokenStar, TokenSlash, TokenPercent:
		return 9
	case TokenColonColon:
		return 10
	case TokenColon:
		return 11
	default:
		return -1
	}
}

func (p *Parser) shouldStopExpressionAtLineBreak() bool {
	if p.multilineExprDepth > 0 || p.pos == 0 || p.isAtEnd() {
		return false
	}
	prev := p.tokens[p.pos-1]
	next := p.peek()
	if prev.EndLine == next.Line {
		return false
	}
	return !isExpressionContinuationToken(prev.Type)
}

func isExpressionContinuationToken(tokenType TokenType) bool {
	switch tokenType {
	case TokenPlus, TokenMinus, TokenStar, TokenSlash, TokenPercent,
		TokenAndAnd, TokenOrOr,
		TokenEqEq, TokenBangEq, TokenLT, TokenLTE, TokenGT, TokenGTE,
		TokenPlusPlus, TokenMinusMinus, TokenColonPlus, TokenColonMinus,
		TokenPipe, TokenAmp, TokenLTLT, TokenGTGT, TokenColonColon,
		TokenFatArrow, TokenComma, TokenDot:
		return true
	default:
		return false
	}
}

func (p *Parser) parseParenExpr(start Token) (Expr, error) {
	if p.check(TokenRParen) {
		end, err := p.consume(TokenRParen, "expected ')'")
		if err != nil {
			return nil, err
		}
		return &UnitLiteral{Span: mergeSpans(tokenSpan(start), tokenSpan(end))}, nil
	}
	p.multilineExprDepth++
	defer func() { p.multilineExprDepth-- }()
	first, err := p.parseExpression(0)
	if err != nil {
		return nil, err
	}
	if !p.match(TokenComma) {
		if _, err := p.consume(TokenRParen, "expected ')'"); err != nil {
			return nil, err
		}
		return &GroupExpr{Inner: first, Span: mergeSpans(tokenSpan(start), tokenSpan(p.previous()))}, nil
	}
	elements := []Expr{first}
	for {
		expr, err := p.parseExpression(0)
		if err != nil {
			return nil, err
		}
		elements = append(elements, expr)
		if !p.match(TokenComma) {
			break
		}
	}
	if _, err := p.consume(TokenRParen, "expected ')'"); err != nil {
		return nil, err
	}
	return &TupleLiteral{Elements: elements, Span: mergeSpans(tokenSpan(start), tokenSpan(p.previous()))}, nil
}

func unaryPrecedence() int {
	return 9
}
