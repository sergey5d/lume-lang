package parser

import "fmt"

func (p *Parser) parseClass() (*ClassDecl, error) {
	return p.parseClassLike(TokenClass, false, false, "class", false)
}

func (p *Parser) parseObject() (*ClassDecl, error) {
	return p.parseClassLike(TokenObject, false, false, "object", true)
}

func (p *Parser) parseRecord() (*ClassDecl, error) {
	return p.parseClassLike(TokenRecord, true, false, "record", false)
}

func (p *Parser) parsePrivateClass() (*ClassDecl, error) {
	return p.parseClassLike(TokenClass, false, true, "class", false)
}

func (p *Parser) parsePrivateObject() (*ClassDecl, error) {
	return p.parseClassLike(TokenObject, false, true, "object", true)
}

func (p *Parser) parsePrivateRecord() (*ClassDecl, error) {
	return p.parseClassLike(TokenRecord, true, true, "record", false)
}

func (p *Parser) parseEnum() (*ClassDecl, error) {
	return p.parseEnumLike(false)
}

func (p *Parser) parsePrivateEnum() (*ClassDecl, error) {
	return p.parseEnumLike(true)
}

func (p *Parser) parseEnumLike(private bool) (*ClassDecl, error) {
	start, err := p.consume(TokenEnum, "expected 'enum'")
	if err != nil {
		return nil, err
	}
	name, err := p.consume(TokenIdentifier, "expected enum name")
	if err != nil {
		return nil, err
	}
	typeParams, err := p.parseTypeParameters()
	if err != nil {
		return nil, err
	}
	decl := &ClassDecl{Name: name.Lexeme, Private: private, Enum: true, TypeParameters: typeParams}
	if p.match(TokenWith) {
		for {
			target, err := p.parseTypeRef()
			if err != nil {
				return nil, err
			}
			decl.Implements = append(decl.Implements, target)
			if !p.match(TokenComma) {
				break
			}
		}
	}
	if _, err := p.consume(TokenLBrace, "expected '{' after enum name"); err != nil {
		return nil, err
	}
	sawNonField := false
	for !p.check(TokenRBrace) && !p.isAtEnd() {
		annotations, err := p.parseAnnotations()
		if err != nil {
			return nil, err
		}
		switch p.peek().Type {
		case TokenIdentifier, TokenVar:
			if sawNonField {
				return nil, fmt.Errorf("enum fields must appear before method or case declarations")
			}
			field, err := p.parseField(annotations, false, false)
			if err != nil {
				return nil, err
			}
			decl.Fields = append(decl.Fields, field)
		case TokenDef, TokenPartial:
			sawNonField = true
			method, err := p.parseMethodLike(annotations, false)
			if err != nil {
				return nil, err
			}
			decl.Methods = append(decl.Methods, method)
		case TokenPub:
			return nil, fmt.Errorf("public is not allowed on enum members")
		case TokenOperator:
			return nil, fmt.Errorf("use 'def %s' instead of 'operator %s' in enum declarations", p.peekNextOperatorExample(), p.peekNextOperatorExample())
		case TokenCase:
			sawNonField = true
			enumCase, err := p.parseEnumCase(annotations)
			if err != nil {
				return nil, err
			}
			decl.Cases = append(decl.Cases, enumCase)
		default:
			return nil, fmt.Errorf("expected enum member, got %s", p.peek().String())
		}
	}
	end, err := p.consume(TokenRBrace, "expected '}' after enum body")
	if err != nil {
		return nil, err
	}
	decl.Span = mergeSpans(tokenSpan(start), tokenSpan(end))
	return decl, nil
}

func (p *Parser) parseClassLike(kind TokenType, record bool, private bool, noun string, allowInlineMethods bool) (*ClassDecl, error) {
	start, err := p.consume(kind, "expected '"+noun+"'")
	if err != nil {
		return nil, err
	}
	name, err := p.consume(TokenIdentifier, "expected "+noun+" name")
	if err != nil {
		return nil, err
	}
	typeParams, err := p.parseTypeParameters()
	if err != nil {
		return nil, err
	}
	decl := &ClassDecl{Name: name.Lexeme, Private: private, Object: kind == TokenObject, Record: record, TypeParameters: typeParams}
	if p.match(TokenWith) {
		for {
			target, err := p.parseTypeRef()
			if err != nil {
				return nil, err
			}
			decl.Implements = append(decl.Implements, target)
			if !p.match(TokenComma) {
				break
			}
		}
	}
	if _, err := p.consume(TokenLBrace, "expected '{' after "+noun+" name"); err != nil {
		return nil, err
	}
	sawMethod := false
	for !p.check(TokenRBrace) && !p.isAtEnd() {
		annotations, err := p.parseAnnotations()
		if err != nil {
			return nil, err
		}
		private := p.match(TokenPrivate)
		if p.check(TokenPub) {
			return nil, fmt.Errorf("public is not allowed on %s members", noun)
		}
		switch p.peek().Type {
		case TokenIdentifier, TokenVar:
			if sawMethod {
				return nil, fmt.Errorf("class fields must appear before method declarations")
			}
			field, err := p.parseField(annotations, private, !record && kind != TokenEnum)
			if err != nil {
				return nil, err
			}
			decl.Fields = append(decl.Fields, field)
		case TokenDef, TokenPartial:
			if !allowInlineMethods {
				return nil, fmt.Errorf("%s methods must be declared in top-level impl blocks", noun)
			}
			sawMethod = true
			method, err := p.parseMethodLike(annotations, private)
			if err != nil {
				return nil, err
			}
			decl.Methods = append(decl.Methods, method)
		case TokenOperator:
			return nil, fmt.Errorf("use symbolic 'def' declarations instead of the 'operator' keyword")
		default:
			return nil, fmt.Errorf("expected class member, got %s", p.peek().String())
		}
	}
	end, err := p.consume(TokenRBrace, "expected '}' after class body")
	if err != nil {
		return nil, err
	}
	decl.Span = mergeSpans(tokenSpan(start), tokenSpan(end))
	return decl, nil
}

func (p *Parser) parseField(annotations []Annotation, private bool, allowPrivateInference bool) (FieldDecl, error) {
	mutable := p.match(TokenVar)
	start, err := p.consume(TokenIdentifier, "expected field name")
	if err != nil {
		return FieldDecl{}, err
	}
	name := start
	var typ *TypeRef
	if !(allowPrivateInference && private && (p.check(TokenAssign) || p.check(TokenColonAssign))) {
		if !private && (p.check(TokenAssign) || p.check(TokenColonAssign)) {
			return FieldDecl{}, fmt.Errorf("member field '%s' requires an explicit type", name.Lexeme)
		}
		typ, err = p.parseTypeRef()
		if err != nil {
			return FieldDecl{}, err
		}
	}
	span := tokenSpan(start)
	if typ != nil {
		span = mergeSpans(span, typeSpan(typ))
	}
	field := FieldDecl{
		Annotations: annotations,
		Name:        name.Lexeme,
		Type:        typ,
		Mutable:     mutable,
		Deferred:    true,
		Private:     private,
		Span:        span,
	}
	switch p.peek().Type {
	case TokenAssign, TokenColonAssign:
		operator := p.advance()
		if operator.Type == TokenColonAssign {
			return FieldDecl{}, fmt.Errorf("cannot use ':=' in a field declaration; use 'var' with '=' for mutable fields")
		}
		if err := p.requireSameLineExpressionStart(operator); err != nil {
			return FieldDecl{}, err
		}
		if p.match(TokenQuestion) {
			if field.Type == nil {
				return FieldDecl{}, fmt.Errorf("private fields with inferred type require an initializer")
			}
			if field.Mutable {
				return FieldDecl{}, fmt.Errorf("mutable fields do not support '= ?'; omit the initializer instead")
			}
			field.Deferred = true
			field.Span = mergeSpans(field.Span, tokenSpan(p.previous()))
			return field, nil
		}
		expr, err := p.parseExpression(0)
		if err != nil {
			return FieldDecl{}, err
		}
		field.Initializer = expr
		field.Deferred = false
		field.Span = mergeSpans(field.Span, exprSpan(expr))
	default:
		if field.Type == nil {
			return FieldDecl{}, fmt.Errorf("private fields with inferred type require an initializer")
		}
	}
	return field, nil
}

func (p *Parser) parseMethodLike(annotations []Annotation, private bool) (*MethodDecl, error) {
	return p.parseMethod(annotations, private)
}

func (p *Parser) parseMethod(annotations []Annotation, private bool) (*MethodDecl, error) {
	start := p.peek()
	partial := false
	switch p.peek().Type {
	case TokenDef:
		if _, err := p.consume(TokenDef, "expected 'def'"); err != nil {
			return nil, err
		}
	case TokenPartial:
		if _, err := p.consume(TokenPartial, "expected 'partial'"); err != nil {
			return nil, err
		}
		partial = true
	default:
		return nil, fmt.Errorf("expected 'def' or 'partial'")
	}
	start = p.previous()
	nameLexeme, isOperator, err := p.parseDeclaredMethodName("expected method name")
	if err != nil {
		return nil, err
	}
	typeParams, err := p.parseTypeParameters()
	if err != nil {
		return nil, err
	}
	params, err := p.parseParameters()
	if err != nil {
		return nil, err
	}
	constructor := nameLexeme == "init"
	var returnType *TypeRef
	if !constructor && !p.check(TokenAssign) && !(p.check(TokenLBrace) && !p.typeRefFollowedBy(TokenAssign)) {
		typ, err := p.parseTypeRef()
		if err != nil {
			return nil, err
		}
		returnType = typ
	}
	var body *BlockStmt
	if partial {
		if constructor {
			return nil, fmt.Errorf("constructors cannot be declared as partial")
		}
		if returnType == nil {
			return nil, fmt.Errorf("partial methods require an explicit return type")
		}
		body, err = p.parsePartialCallableBody(start, params)
		if err != nil {
			return nil, err
		}
		returnType = optionTypeRef(returnType)
	} else {
		body, err = p.parseCallableBody()
		if err != nil {
			return nil, err
		}
	}
	if !constructor && returnType == nil {
		returnType = implicitUnitType(body.Span)
	}
	return &MethodDecl{
		Annotations:    annotations,
		Name:           nameLexeme,
		TypeParameters: typeParams,
		Parameters:     params,
		ReturnType:     returnType,
		Body:           body,
		Partial:        partial,
		Operator:       isOperator,
		Private:        private,
		Constructor:    constructor,
		Span:           mergeSpans(tokenSpan(start), body.Span),
	}, nil
}

func optionTypeRef(inner *TypeRef) *TypeRef {
	if inner == nil {
		return &TypeRef{Name: "Option"}
	}
	return &TypeRef{
		Name:      "Option",
		Arguments: []*TypeRef{inner},
		Span:      inner.Span,
	}
}

func (p *Parser) parsePartialCallableBody(start Token, params []Parameter) (*BlockStmt, error) {
	if len(params) == 0 {
		return nil, fmt.Errorf("partial methods require at least one parameter")
	}
	cases, end, err := p.parseMatchCases()
	if err != nil {
		return nil, err
	}
	matchExpr := &MatchExpr{
		Partial: true,
		Value:   partialCallableValueExpr(params),
		Cases:   cases,
		Span:    mergeSpans(tokenSpan(start), tokenSpan(end)),
	}
	stmt := &ExprStmt{Expr: matchExpr, Span: exprSpan(matchExpr)}
	return &BlockStmt{
		Statements: []Statement{stmt},
		Span:       exprSpan(matchExpr),
	}, nil
}

func partialCallableValueExpr(params []Parameter) Expr {
	if len(params) == 1 {
		return &Identifier{Name: params[0].Name, Span: params[0].Span}
	}
	elements := make([]Expr, len(params))
	span := params[0].Span
	for i, param := range params {
		elements[i] = &Identifier{Name: param.Name, Span: param.Span}
		span = mergeSpans(span, param.Span)
	}
	return &TupleLiteral{Elements: elements, Span: span}
}

func (p *Parser) parseOperatorName() (string, Span, error) {
	token := p.advance()
	switch token.Type {
	case TokenPlus, TokenMinus, TokenStar, TokenSlash, TokenPercent:
		return token.Lexeme, tokenSpan(token), nil
	case TokenColonPlus:
		return ":+", tokenSpan(token), nil
	case TokenColonMinus:
		return ":-", tokenSpan(token), nil
	case TokenPlusPlus:
		return "++", tokenSpan(token), nil
	case TokenMinusMinus:
		return "--", tokenSpan(token), nil
	case TokenPipe:
		return "|", tokenSpan(token), nil
	case TokenAmp:
		return "&", tokenSpan(token), nil
	case TokenGTGT:
		return ">>", tokenSpan(token), nil
	case TokenLTLT:
		return "<<", tokenSpan(token), nil
	case TokenTilde:
		return "~", tokenSpan(token), nil
	case TokenColonColon:
		return "::", tokenSpan(token), nil
	case TokenLBracket:
		end, err := p.consume(TokenRBracket, "expected ']' after operator '['")
		if err != nil {
			return "", Span{}, err
		}
		return "[]", mergeSpans(tokenSpan(token), tokenSpan(end)), nil
	default:
		return "", Span{}, fmt.Errorf("expected operator symbol, got %s", token.String())
	}
}

func (p *Parser) parseDeclaredMethodName(identifierMessage string) (string, bool, error) {
	if p.check(TokenIdentifier) {
		name, err := p.consume(TokenIdentifier, identifierMessage)
		if err != nil {
			return "", false, err
		}
		return name.Lexeme, false, nil
	}
	if isOperatorNameToken(p.peek().Type) {
		name, _, err := p.parseOperatorName()
		if err != nil {
			return "", false, err
		}
		return name, true, nil
	}
	name, err := p.consume(TokenIdentifier, identifierMessage)
	if err != nil {
		return "", false, err
	}
	return name.Lexeme, false, nil
}

func isOperatorNameToken(tokenType TokenType) bool {
	switch tokenType {
	case TokenPlus, TokenMinus, TokenStar, TokenSlash, TokenPercent, TokenColonPlus, TokenColonMinus, TokenPlusPlus, TokenMinusMinus, TokenPipe, TokenAmp, TokenGTGT, TokenLTLT, TokenTilde, TokenColonColon, TokenLBracket:
		return true
	default:
		return false
	}
}

func (p *Parser) peekNextOperatorExample() string {
	if p.pos+1 >= len(p.tokens) {
		return "<op>"
	}
	next := p.tokens[p.pos+1]
	if next.Type == TokenLBracket && p.pos+2 < len(p.tokens) && p.tokens[p.pos+2].Type == TokenRBracket {
		return "[]"
	}
	if next.Lexeme != "" {
		return next.Lexeme
	}
	return "<op>"
}

func (p *Parser) parseEnumCase(annotations []Annotation) (EnumCaseDecl, error) {
	start, err := p.consume(TokenCase, "expected 'case'")
	if err != nil {
		return EnumCaseDecl{}, err
	}
	name, err := p.consume(TokenIdentifier, "expected case name")
	if err != nil {
		return EnumCaseDecl{}, err
	}
	decl := EnumCaseDecl{Annotations: annotations, Name: name.Lexeme, Span: mergeSpans(tokenSpan(start), tokenSpan(name))}
	if !p.match(TokenLBrace) {
		return decl, nil
	}
	for !p.check(TokenRBrace) && !p.isAtEnd() {
		if p.check(TokenIdentifier) && (p.checkNext(TokenAssign) || p.checkNext(TokenColonAssign)) {
			assignStart := p.advance()
			op := p.advance()
			if op.Type != TokenAssign {
				return EnumCaseDecl{}, fmt.Errorf("enum case field assignments must use '='")
			}
			if err := p.requireSameLineExpressionStart(op); err != nil {
				return EnumCaseDecl{}, err
			}
			value, err := p.parseExpression(0)
			if err != nil {
				return EnumCaseDecl{}, err
			}
			decl.Assignments = append(decl.Assignments, EnumCaseAssignment{
				Name:  assignStart.Lexeme,
				Value: value,
				Span:  mergeSpans(tokenSpan(assignStart), exprSpan(value)),
			})
			continue
		}
		field, err := p.parseField(nil, false, false)
		if err != nil {
			return EnumCaseDecl{}, err
		}
		decl.Fields = append(decl.Fields, field)
	}
	end, err := p.consume(TokenRBrace, "expected '}' after case body")
	if err != nil {
		return EnumCaseDecl{}, err
	}
	decl.Span = mergeSpans(tokenSpan(start), tokenSpan(end))
	return decl, nil
}

func (p *Parser) parseTopLevelImpl(program *Program) error {
	start, err := p.consume(TokenImpl, "expected 'impl'")
	if err != nil {
		return err
	}
	target, err := p.parseTypeRef()
	if err != nil {
		return err
	}
	var targetClass *ClassDecl
	for i := len(program.Classes) - 1; i >= 0; i-- {
		if program.Classes[i].Name == target.Name {
			targetClass = program.Classes[i]
			break
		}
	}
	if targetClass == nil {
		return fmt.Errorf("unknown impl target '%s'", target.Name)
	}
	if p.match(TokenDot) {
		caseName, err := p.consume(TokenIdentifier, "expected enum case name after '.'")
		if err != nil {
			return err
		}
		if !targetClass.Enum {
			return fmt.Errorf("impl target '%s.%s' requires enum '%s'", target.Name, caseName.Lexeme, target.Name)
		}
		foundCase := false
		for i := range targetClass.Cases {
			if targetClass.Cases[i].Name == caseName.Lexeme {
				foundCase = true
				break
			}
		}
		if !foundCase {
			return fmt.Errorf("unknown enum case '%s.%s'", target.Name, caseName.Lexeme)
		}
		return fmt.Errorf("enum cases cannot declare methods; move methods to enum '%s'", target.Name)
	}
	if _, err := p.consume(TokenLBrace, "expected '{' after impl target"); err != nil {
		return err
	}
	for !p.check(TokenRBrace) && !p.isAtEnd() {
		annotations, err := p.parseAnnotations()
		if err != nil {
			return err
		}
		private := p.match(TokenPrivate)
		if !p.check(TokenDef) && !p.check(TokenPartial) {
			return fmt.Errorf("expected method declaration in impl block, got %s", p.peek().String())
		}
		method, err := p.parseMethodLike(annotations, private)
		if err != nil {
			return err
		}
		targetClass.Methods = append(targetClass.Methods, method)
	}
	end, err := p.consume(TokenRBrace, "expected '}' after impl body")
	if err != nil {
		return err
	}
	targetClass.Span = mergeSpans(targetClass.Span, tokenSpan(end))
	_ = start
	return nil
}
