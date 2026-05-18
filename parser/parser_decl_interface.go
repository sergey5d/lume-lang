package parser

import "fmt"

func (p *Parser) parseInterface() (*InterfaceDecl, error) {
	return p.parseInterfaceWithPrivate(false)
}

func (p *Parser) parsePrivateInterface() (*InterfaceDecl, error) {
	return p.parseInterfaceWithPrivate(true)
}

func (p *Parser) parseInterfaceWithPrivate(private bool) (*InterfaceDecl, error) {
	start, err := p.consume(TokenInterface, "expected 'interface'")
	if err != nil {
		return nil, err
	}
	name, err := p.consume(TokenIdentifier, "expected interface name")
	if err != nil {
		return nil, err
	}
	typeParams, err := p.parseTypeParameters()
	if err != nil {
		return nil, err
	}
	decl := &InterfaceDecl{Name: name.Lexeme, Private: private, TypeParameters: typeParams}
	if p.match(TokenWith) {
		for {
			target, err := p.parseTypeRef()
			if err != nil {
				return nil, err
			}
			decl.Extends = append(decl.Extends, target)
			if !p.match(TokenComma) {
				break
			}
		}
	}
	if _, err := p.consume(TokenLBrace, "expected '{' after interface name"); err != nil {
		return nil, err
	}
	for !p.check(TokenRBrace) && !p.isAtEnd() {
		annotations, err := p.parseAnnotations()
		if err != nil {
			return nil, err
		}
		if p.check(TokenPub) {
			return nil, fmt.Errorf("public is not allowed inside interfaces")
		}
		if p.check(TokenOperator) {
			return nil, fmt.Errorf("use symbolic 'def' declarations instead of the 'operator' keyword")
		}
		method, err := p.parseInterfaceMethod(annotations)
		if err != nil {
			return nil, err
		}
		decl.Methods = append(decl.Methods, method)
	}
	end, err := p.consume(TokenRBrace, "expected '}' after interface body")
	if err != nil {
		return nil, err
	}
	decl.Span = mergeSpans(tokenSpan(start), tokenSpan(end))
	return decl, nil
}

func (p *Parser) parseInterfaceMethod(annotations []Annotation) (InterfaceMethod, error) {
	start, err := p.consume(TokenDef, "expected 'def' in interface")
	if err != nil {
		return InterfaceMethod{}, err
	}
	nameLexeme, _, err := p.parseDeclaredMethodName("expected method name")
	if err != nil {
		return InterfaceMethod{}, err
	}
	typeParams, err := p.parseTypeParameters()
	if err != nil {
		return InterfaceMethod{}, err
	}
	params, err := p.parseParameters()
	if err != nil {
		return InterfaceMethod{}, err
	}
	var returnType *TypeRef
	var body *BlockStmt
	if p.check(TokenAssign) || (p.check(TokenLBrace) && !p.typeRefFollowedBy(TokenAssign)) {
		body, err = p.parseCallableBody()
		if err != nil {
			return InterfaceMethod{}, err
		}
		returnType = implicitUnitType(body.Span)
	} else if p.check(TokenDef) || p.check(TokenRBrace) {
		returnType = implicitUnitType(tokenSpan(p.previous()))
	} else {
		returnType, err = p.parseTypeRef()
		if err != nil {
			return InterfaceMethod{}, err
		}
		if p.check(TokenAssign) || (p.check(TokenLBrace) && !p.typeRefFollowedBy(TokenAssign)) {
			body, err = p.parseCallableBody()
			if err != nil {
				return InterfaceMethod{}, err
			}
		}
	}
	return InterfaceMethod{
		Annotations:    annotations,
		Name:           nameLexeme,
		TypeParameters: typeParams,
		Parameters:     params,
		ReturnType:     returnType,
		Body:           body,
		Span:           mergeSpans(tokenSpan(start), endSpanOrType(returnType, body)),
	}, nil
}

func endSpanOrType(returnType *TypeRef, body *BlockStmt) Span {
	if body != nil {
		return body.Span
	}
	return typeSpan(returnType)
}
