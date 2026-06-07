package parser

import "fmt"

func (p *Parser) parseLambdaIdentifier() (Expr, error) {
	param, err := p.parseLambdaParameter()
	if err != nil {
		return nil, err
	}
	if (p.check(TokenIdentifier) || p.check(TokenLBracket)) && p.simpleTypeRefFollowedBy(TokenArrow) {
		typeRef, err := p.parsePrimaryTypeRef()
		if err != nil {
			return nil, err
		}
		param.Type = typeRef
		param.Span = mergeSpans(param.Span, typeSpan(typeRef))
	}
	if _, err := p.consume(TokenArrow, "expected '->' after lambda parameter"); err != nil {
		return nil, err
	}
	body, blockBody, endSpan, err := p.parseLambdaBody()
	if err != nil {
		return nil, err
	}
	return &LambdaExpr{
		Parameters: []LambdaParameter{param},
		Body:       body,
		BlockBody:  blockBody,
		Span:       mergeSpans(param.Span, endSpan),
	}, nil
}

func (p *Parser) parseLambdaParen() (Expr, error) {
	params, err := p.parseLambdaParams()
	if err != nil {
		return nil, err
	}
	if _, err := p.consume(TokenArrow, "expected '->' after lambda parameters"); err != nil {
		return nil, err
	}
	body, blockBody, endSpan, err := p.parseLambdaBody()
	if err != nil {
		return nil, err
	}
	startSpan := Span{}
	if len(params) > 0 {
		startSpan = params[0].Span
	}
	return &LambdaExpr{Parameters: params, Body: body, BlockBody: blockBody, Span: mergeSpans(startSpan, endSpan)}, nil
}

func (p *Parser) parseLambdaBody() (Expr, *BlockStmt, Span, error) {
	if p.check(TokenLBrace) {
		block, err := p.parseBlock()
		if err != nil {
			return nil, nil, Span{}, err
		}
		return nil, block, block.Span, nil
	}
	body, err := p.parseExpression(0)
	if err != nil {
		return nil, nil, Span{}, err
	}
	return body, nil, exprSpan(body), nil
}

func (p *Parser) isBraceLambdaStart() bool {
	if !p.check(TokenLBrace) {
		return false
	}
	start := p.pos
	p.advance()
	defer func() { p.pos = start }()
	return p.isLambdaIdentifierStart() || p.isLambdaParenStart()
}

func (p *Parser) parseBraceLambdaExpr() (Expr, error) {
	start, err := p.consume(TokenLBrace, "expected '{'")
	if err != nil {
		return nil, err
	}
	var (
		params  []LambdaParameter
		endSpan Span
	)
	switch {
	case p.isLambdaIdentifierStart():
		param, err := p.parseLambdaParameter()
		if err != nil {
			return nil, err
		}
		if (p.check(TokenIdentifier) || p.check(TokenLBracket)) && p.simpleTypeRefFollowedBy(TokenArrow) {
			typeRef, err := p.parsePrimaryTypeRef()
			if err != nil {
				return nil, err
			}
			param.Type = typeRef
			param.Span = mergeSpans(param.Span, typeSpan(typeRef))
		}
		params = []LambdaParameter{param}
	case p.isLambdaParenStart():
		params, err = p.parseLambdaParams()
		if err != nil {
			return nil, err
		}
	default:
		return nil, fmt.Errorf("expected lambda parameters after '{' at %d:%d", start.Line, start.Column)
	}
	if _, err := p.consume(TokenArrow, "expected '->' after lambda parameters"); err != nil {
		return nil, err
	}
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
	end, err := p.consume(TokenRBrace, "expected '}' after lambda block")
	if err != nil {
		return nil, err
	}
	block.Span = mergeSpans(tokenSpan(start), tokenSpan(end))
	endSpan = tokenSpan(end)
	startSpan := tokenSpan(start)
	if len(params) > 0 {
		startSpan = params[0].Span
	}
	return &LambdaExpr{
		Parameters: params,
		BlockBody:  block,
		Span:       mergeSpans(startSpan, endSpan),
	}, nil
}

func (p *Parser) parseLambdaParams() ([]LambdaParameter, error) {
	if _, err := p.consume(TokenLParen, "expected '('"); err != nil {
		return nil, err
	}
	var params []LambdaParameter
	if !p.check(TokenRParen) {
		for {
			param, err := p.parseLambdaParameter()
			if err != nil {
				return nil, err
			}
			lambdaParam := param
			if (p.check(TokenIdentifier) || p.check(TokenLParen) || p.check(TokenLBrace) || p.check(TokenLBracket)) && (p.typeRefFollowedBy(TokenComma) || p.typeRefFollowedBy(TokenRParen)) {
				typeRef, err := p.parseTypeRef()
				if err != nil {
					return nil, err
				}
				lambdaParam.Type = typeRef
				lambdaParam.Span = mergeSpans(lambdaParam.Span, typeSpan(typeRef))
			}
			params = append(params, lambdaParam)
			if !p.match(TokenComma) {
				break
			}
		}
	}
	if _, err := p.consume(TokenRParen, "expected ')' after lambda parameters"); err != nil {
		return nil, err
	}
	return params, nil
}

func (p *Parser) isLambdaIdentifierStart() bool {
	if !p.check(TokenIdentifier) && !p.check(TokenUnder) {
		return false
	}
	if p.checkNext(TokenArrow) {
		return true
	}
	if p.check(TokenIdentifier) {
		return (p.checkNext(TokenIdentifier) || p.checkNext(TokenLBracket)) && p.simpleTypeRefFollowedByAt(p.pos+1, TokenArrow)
	}
	return false
}

func (p *Parser) isLambdaParenStart() bool {
	if !p.check(TokenLParen) {
		return false
	}
	i := p.pos + 1
	if p.tokens[p.pos].Type != TokenLParen {
		return false
	}
	if i >= len(p.tokens) {
		return false
	}
	if p.tokens[i].Type == TokenRParen {
		return i+1 < len(p.tokens) && p.tokens[i+1].Type == TokenArrow
	}
	for {
		if i >= len(p.tokens) || !isLambdaParamToken(p.tokens[i].Type) {
			return false
		}
		i++
		if i < len(p.tokens) && p.tokens[i].Type == TokenIdentifier && p.tokens[i-1].Type != TokenUnder {
			end, ok := p.scanTypeRef(i)
			if !ok {
				return false
			}
			i = end
		}
		if i >= len(p.tokens) {
			return false
		}
		if p.tokens[i].Type == TokenComma {
			i++
			continue
		}
		if p.tokens[i].Type == TokenRParen {
			return i+1 < len(p.tokens) && p.tokens[i+1].Type == TokenArrow
		}
		return false
	}
}

func (p *Parser) parseLambdaParameter() (LambdaParameter, error) {
	var param Token
	var err error
	switch {
	case p.check(TokenIdentifier):
		param, err = p.consume(TokenIdentifier, "expected lambda parameter")
	case p.check(TokenUnder):
		param, err = p.consume(TokenUnder, "expected lambda parameter")
	default:
		return LambdaParameter{}, fmt.Errorf("expected lambda parameter, got %s", p.peek().String())
	}
	if err != nil {
		return LambdaParameter{}, err
	}
	return LambdaParameter{Name: param.Lexeme, Span: tokenSpan(param)}, nil
}

func isLambdaParamToken(tokenType TokenType) bool {
	return tokenType == TokenIdentifier || tokenType == TokenUnder
}
