package parser

import "fmt"

func (p *Parser) parseTypeRef() (*TypeRef, error) {
	left, err := p.parsePrimaryTypeRef()
	if err != nil {
		return nil, err
	}
	if p.match(TokenArrow) {
		returnType, err := p.parseTypeRef()
		if err != nil {
			return nil, err
		}
		return &TypeRef{
			ParameterTypes: []*TypeRef{left},
			ReturnType:     returnType,
			Span:           mergeSpans(left.Span, typeSpan(returnType)),
		}, nil
	}
	return left, nil
}

func (p *Parser) parsePrimaryTypeRef() (*TypeRef, error) {
	if p.check(TokenLParen) {
		return p.parseParenTypeRef()
	}
	if p.check(TokenLBrace) {
		return p.parseRecordTypeRef()
	}
	if p.check(TokenLBracket) {
		return p.parseListTypeRef()
	}
	return p.parseNamedTypeRef()
}

func (p *Parser) parseNamedTypeRef() (*TypeRef, error) {
	name, err := p.consume(TokenIdentifier, "expected type name")
	if err != nil {
		return nil, err
	}
	ref := &TypeRef{Name: name.Lexeme, Span: tokenSpan(name)}
	if p.match(TokenLBracket) {
		for {
			arg, err := p.parseTypeRef()
			if err != nil {
				return nil, err
			}
			ref.Arguments = append(ref.Arguments, arg)
			if !p.match(TokenComma) {
				break
			}
		}
		end, err := p.consume(TokenRBracket, "expected ']' after type arguments")
		if err != nil {
			return nil, err
		}
		ref.Span = mergeSpans(ref.Span, tokenSpan(end))
	}
	return ref, nil
}

func (p *Parser) parseListTypeRef() (*TypeRef, error) {
	start, err := p.consume(TokenLBracket, "expected '[' before list type")
	if err != nil {
		return nil, err
	}
	element, err := p.parseTypeRef()
	if err != nil {
		return nil, err
	}
	end, err := p.consume(TokenRBracket, "expected ']' after list type")
	if err != nil {
		return nil, err
	}
	return &TypeRef{
		Name:      "List",
		Arguments: []*TypeRef{element},
		Span:      mergeSpans(tokenSpan(start), tokenSpan(end)),
	}, nil
}

func (p *Parser) parseParenTypeRef() (*TypeRef, error) {
	start, err := p.consume(TokenLParen, "expected '('")
	if err != nil {
		return nil, err
	}
	var params []*TypeRef
	if !p.check(TokenRParen) {
		for {
			param, err := p.parseTypeRef()
			if err != nil {
				return nil, err
			}
			params = append(params, param)
			if !p.match(TokenComma) {
				break
			}
		}
	}
	if _, err := p.consume(TokenRParen, "expected ')' after function type parameters"); err != nil {
		return nil, err
	}
	if !p.match(TokenArrow) {
		if len(params) == 1 {
			return params[0], nil
		}
		return &TypeRef{
			Name:          "Tuple",
			TupleElements: params,
			Span:          mergeSpans(tokenSpan(start), tokenSpan(p.previous())),
		}, nil
	}
	returnType, err := p.parseTypeRef()
	if err != nil {
		return nil, err
	}
	return &TypeRef{
		ParameterTypes: params,
		ReturnType:     returnType,
		Span:           mergeSpans(tokenSpan(start), typeSpan(returnType)),
	}, nil
}

func (p *Parser) parseRecordTypeRef() (*TypeRef, error) {
	start, err := p.consume(TokenLBrace, "expected '{'")
	if err != nil {
		return nil, err
	}
	if p.check(TokenRBrace) {
		return nil, fmt.Errorf("anonymous record type must declare at least one field at %d:%d", start.Line, start.Column)
	}
	var fields []TypeField
	for {
		name, err := p.consume(TokenIdentifier, "expected record field name")
		if err != nil {
			return nil, err
		}
		typ, err := p.parseTypeRef()
		if err != nil {
			return nil, err
		}
		fields = append(fields, TypeField{
			Name: name.Lexeme,
			Type: typ,
			Span: mergeSpans(tokenSpan(name), typeSpan(typ)),
		})
		if !p.match(TokenComma) {
			break
		}
	}
	end, err := p.consume(TokenRBrace, "expected '}' after anonymous record type")
	if err != nil {
		return nil, err
	}
	return &TypeRef{
		Name:         "Record",
		RecordFields: fields,
		Span:         mergeSpans(tokenSpan(start), tokenSpan(end)),
	}, nil
}

func (p *Parser) typeRefFollowedBy(tt TokenType) bool {
	return p.typeRefFollowedByAt(p.pos, tt)
}

func (p *Parser) simpleTypeRefFollowedBy(tt TokenType) bool {
	return p.simpleTypeRefFollowedByAt(p.pos, tt)
}

func (p *Parser) typeRefFollowedByAt(start int, tt TokenType) bool {
	end, ok := p.scanTypeRef(start)
	if !ok || end >= len(p.tokens) {
		return false
	}
	return p.tokens[end].Type == tt
}

func (p *Parser) simpleTypeRefFollowedByAt(start int, tt TokenType) bool {
	end, ok := p.scanSimpleTypeRef(start)
	if !ok || end >= len(p.tokens) {
		return false
	}
	return p.tokens[end].Type == tt
}

func (p *Parser) scanTypeRef(start int) (int, bool) {
	if start >= len(p.tokens) {
		return start, false
	}
	if p.tokens[start].Type == TokenLBracket {
		i := start + 1
		var ok bool
		i, ok = p.scanTypeRef(i)
		if !ok || i >= len(p.tokens) || p.tokens[i].Type != TokenRBracket {
			return start, false
		}
		i++
		if i < len(p.tokens) && p.tokens[i].Type == TokenArrow {
			return p.scanTypeRef(i + 1)
		}
		return i, true
	}
	if p.tokens[start].Type == TokenLBrace {
		i := start + 1
		if i >= len(p.tokens) || p.tokens[i].Type == TokenRBrace {
			return start, false
		}
		for {
			if i >= len(p.tokens) || p.tokens[i].Type != TokenIdentifier {
				return start, false
			}
			i++
			var ok bool
			i, ok = p.scanTypeRef(i)
			if !ok || i >= len(p.tokens) {
				return start, false
			}
			if p.tokens[i].Type == TokenComma {
				i++
				continue
			}
			if p.tokens[i].Type == TokenRBrace {
				return i + 1, true
			}
			return start, false
		}
	}
	if p.tokens[start].Type == TokenLParen {
		i := start + 1
		if i < len(p.tokens) && p.tokens[i].Type != TokenRParen {
			for {
				var ok bool
				i, ok = p.scanTypeRef(i)
				if !ok || i >= len(p.tokens) {
					return start, false
				}
				if p.tokens[i].Type == TokenComma {
					i++
					continue
				}
				if p.tokens[i].Type == TokenRParen {
					i++
					break
				}
				return start, false
			}
		} else if i < len(p.tokens) && p.tokens[i].Type == TokenRParen {
			i++
		} else {
			return start, false
		}
		if i < len(p.tokens) && p.tokens[i].Type == TokenArrow {
			return p.scanTypeRef(i + 1)
		}
		return i, true
	}
	if p.tokens[start].Type != TokenIdentifier {
		return start, false
	}
	i := start + 1
	if i < len(p.tokens) && p.tokens[i].Type == TokenLBracket {
		i++
		for {
			var ok bool
			i, ok = p.scanTypeRef(i)
			if !ok {
				return start, false
			}
			if i >= len(p.tokens) {
				return start, false
			}
			if p.tokens[i].Type == TokenComma {
				i++
				continue
			}
			if p.tokens[i].Type == TokenRBracket {
				i++
				break
			}
			return start, false
		}
	}
	if i < len(p.tokens) && p.tokens[i].Type == TokenArrow {
		return p.scanTypeRef(i + 1)
	}
	return i, true
}

func (p *Parser) scanSimpleTypeRef(start int) (int, bool) {
	if start >= len(p.tokens) {
		return start, false
	}
	if p.tokens[start].Type == TokenLBracket {
		i := start + 1
		var ok bool
		i, ok = p.scanTypeRef(i)
		if !ok || i >= len(p.tokens) || p.tokens[i].Type != TokenRBracket {
			return start, false
		}
		return i + 1, true
	}
	if p.tokens[start].Type != TokenIdentifier {
		return start, false
	}
	i := start + 1
	if i < len(p.tokens) && p.tokens[i].Type == TokenLBracket {
		i++
		for {
			var ok bool
			i, ok = p.scanTypeRef(i)
			if !ok || i >= len(p.tokens) {
				return start, false
			}
			if p.tokens[i].Type == TokenComma {
				i++
				continue
			}
			if p.tokens[i].Type == TokenRBracket {
				i++
				break
			}
			return start, false
		}
	}
	return i, true
}
