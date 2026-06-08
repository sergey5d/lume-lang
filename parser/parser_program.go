package parser

import "fmt"

func (p *Parser) parseProgram() (*Program, error) {
	program := &Program{}
	if p.match(TokenModule) {
		keyword := p.previous()
		name, span, err := p.parseModulePath("expected module name")
		if err != nil {
			return nil, err
		}
		program.ModuleName = name
		program.ModuleSpan = mergeSpans(tokenSpan(keyword), span)
	}
	for p.match(TokenImport) {
		keyword := p.previous()
		imp, err := p.parseImportDecl(keyword)
		if err != nil {
			return nil, err
		}
		program.Imports = append(program.Imports, *imp)
	}
	for !p.isAtEnd() {
		annotations, err := p.parseAnnotations()
		if err != nil {
			return nil, err
		}
		switch p.peek().Type {
		case TokenModule:
			return nil, fmt.Errorf("'module' must appear before declarations")
		case TokenImport:
			return nil, fmt.Errorf("'import' must appear before declarations")
		case TokenDef:
			fn, err := p.parseFunction()
			if err != nil {
				return nil, err
			}
			fn.Annotations = annotations
			program.Functions = append(program.Functions, fn)
		case TokenInterface:
			decl, err := p.parseInterface()
			if err != nil {
				return nil, err
			}
			decl.Annotations = annotations
			program.Interfaces = append(program.Interfaces, decl)
		case TokenClass:
			decl, err := p.parseClass()
			if err != nil {
				return nil, err
			}
			decl.Annotations = annotations
			program.Classes = append(program.Classes, decl)
		case TokenObject:
			decl, err := p.parseObject()
			if err != nil {
				return nil, err
			}
			decl.Annotations = annotations
			program.Classes = append(program.Classes, decl)
		case TokenRecord:
			return nil, fmt.Errorf("named 'record' declarations were removed; use 'class'")
		case TokenEnum:
			decl, err := p.parseEnum()
			if err != nil {
				return nil, err
			}
			decl.Annotations = annotations
			program.Classes = append(program.Classes, decl)
		case TokenImpl:
			if len(annotations) > 0 {
				return nil, fmt.Errorf("annotations are not supported on impl blocks")
			}
			if err := p.parseTopLevelImpl(program); err != nil {
				return nil, err
			}
		case TokenPrivate:
			p.advance()
			switch p.peek().Type {
			case TokenDef:
				fn, err := p.parsePrivateFunction()
				if err != nil {
					return nil, err
				}
				fn.Annotations = annotations
				program.Functions = append(program.Functions, fn)
			case TokenInterface:
				decl, err := p.parsePrivateInterface()
				if err != nil {
					return nil, err
				}
				decl.Annotations = annotations
				program.Interfaces = append(program.Interfaces, decl)
			case TokenClass:
				decl, err := p.parsePrivateClass()
				if err != nil {
					return nil, err
				}
				decl.Annotations = annotations
				program.Classes = append(program.Classes, decl)
			case TokenObject:
				decl, err := p.parsePrivateObject()
				if err != nil {
					return nil, err
				}
				decl.Annotations = annotations
				program.Classes = append(program.Classes, decl)
			case TokenRecord:
				return nil, fmt.Errorf("named 'record' declarations were removed; use 'class'")
			case TokenEnum:
				decl, err := p.parsePrivateEnum()
				if err != nil {
					return nil, err
				}
				decl.Annotations = annotations
				program.Classes = append(program.Classes, decl)
			default:
				return nil, fmt.Errorf("'hidden' is only supported for top-level declarations")
			}
		case TokenPub:
			p.advance()
			switch p.peek().Type {
			case TokenDef:
				fn, err := p.parsePublicFunction()
				if err != nil {
					return nil, err
				}
				fn.Annotations = annotations
				program.Functions = append(program.Functions, fn)
			case TokenInterface, TokenClass, TokenObject, TokenEnum, TokenImpl:
				return nil, fmt.Errorf("'public' is only supported for top-level functions and immutable bindings")
			default:
				if len(annotations) > 0 {
					return nil, fmt.Errorf("annotations are only supported on top-level declarations")
				}
				stmt, err := p.parseStatement()
				if err != nil {
					return nil, err
				}
				valStmt, ok := stmt.(*ValStmt)
				if !ok {
					return nil, fmt.Errorf("'public' is only supported for top-level functions and immutable bindings")
				}
				for _, binding := range valStmt.Bindings {
					if binding.Name == "_" || binding.Mutable {
						return nil, fmt.Errorf("'public' is only supported for immutable named top-level bindings")
					}
				}
				valStmt.Public = true
				program.Statements = append(program.Statements, valStmt)
			}
		default:
			if len(annotations) > 0 {
				return nil, fmt.Errorf("annotations are only supported on top-level declarations")
			}
			stmt, err := p.parseStatement()
			if err != nil {
				return nil, err
			}
			program.Statements = append(program.Statements, stmt)
		}
	}
	if span, ok := p.programSpan(program); ok {
		program.Span = span
	}
	return program, nil
}

func (p *Parser) programSpan(program *Program) (Span, bool) {
	var spans []Span
	if program.ModuleName != "" {
		spans = append(spans, program.ModuleSpan)
	}
	for _, imp := range program.Imports {
		spans = append(spans, imp.Span)
	}
	for _, fn := range program.Functions {
		spans = append(spans, fn.Span)
	}
	for _, decl := range program.Interfaces {
		spans = append(spans, decl.Span)
	}
	for _, decl := range program.Classes {
		spans = append(spans, decl.Span)
	}
	for _, stmt := range program.Statements {
		spans = append(spans, stmtSpan(stmt))
	}
	if len(spans) == 0 {
		return Span{}, false
	}
	return mergeSpans(spans[0], spans[len(spans)-1]), true
}

func (p *Parser) parseModulePath(message string) (string, Span, error) {
	start, err := p.consume(TokenIdentifier, message)
	if err != nil {
		return "", Span{}, err
	}
	path := start.Lexeme
	span := tokenSpan(start)
	for p.match(TokenSlash) {
		next, err := p.consume(TokenIdentifier, "expected path segment after '/'")
		if err != nil {
			return "", Span{}, err
		}
		path += "/" + next.Lexeme
		span = mergeSpans(span, tokenSpan(next))
	}
	return path, span, nil
}

func (p *Parser) parseImportDecl(keyword Token) (*ImportDecl, error) {
	segments, span, err := p.parseImportSegments("expected import path")
	if err != nil {
		return nil, err
	}
	if len(segments) == 0 {
		return nil, fmt.Errorf("expected import path")
	}
	imp := &ImportDecl{}
	switch {
	case p.match(TokenSlash):
		switch {
		case p.match(TokenStar):
			imp.Path = joinImportSegments(segments)
			imp.Wildcard = true
			span = mergeSpans(span, tokenSpan(p.previous()))
		case p.match(TokenLBrace):
			imp.Path = joinImportSegments(segments)
			symbols, symbolsSpan, err := p.parseImportSymbolList()
			if err != nil {
				return nil, err
			}
			imp.Symbols = symbols
			span = mergeSpans(span, symbolsSpan)
		default:
			next, err := p.consume(TokenIdentifier, "expected import symbol, '*', or '{' after '/'")
			if err != nil {
				return nil, err
			}
			imp.Path = joinImportSegments(segments)
			if p.match(TokenSlash) {
				imp.ObjectName = next.Lexeme
				switch {
				case p.match(TokenStar):
					imp.Wildcard = true
					span = mergeSpans(span, tokenSpan(p.previous()))
				case p.match(TokenLBrace):
					symbols, symbolsSpan, err := p.parseImportSymbolList()
					if err != nil {
						return nil, err
					}
					imp.Symbols = symbols
					span = mergeSpans(span, symbolsSpan)
				default:
					return nil, fmt.Errorf("expected object member import '*', or '{' after '%s/'", next.Lexeme)
				}
				span = mergeSpans(span, tokenSpan(next))
				break
			}
			symbol := ImportSymbol{Name: next.Lexeme, Span: tokenSpan(next)}
			if p.match(TokenAs) {
				alias, err := p.consume(TokenIdentifier, "expected alias after 'as'")
				if err != nil {
					return nil, err
				}
				symbol.Alias = alias.Lexeme
				symbol.Span = mergeSpans(symbol.Span, tokenSpan(alias))
			}
			imp.Symbols = []ImportSymbol{symbol}
			span = mergeSpans(span, symbol.Span)
		}
	default:
		imp.Path = joinImportSegments(segments)
	}
	imp.Span = mergeSpans(tokenSpan(keyword), span)
	return imp, nil
}

func (p *Parser) parseImportSegments(message string) ([]string, Span, error) {
	start, err := p.consume(TokenIdentifier, message)
	if err != nil {
		return nil, Span{}, err
	}
	segments := []string{start.Lexeme}
	span := tokenSpan(start)
	for p.check(TokenSlash) && p.checkNth(1, TokenIdentifier) && startsLower(p.tokens[p.pos+1].Lexeme) {
		p.advance()
		next := p.advance()
		segments = append(segments, next.Lexeme)
		span = mergeSpans(span, tokenSpan(next))
	}
	return segments, span, nil
}

func startsLower(s string) bool {
	if s == "" {
		return false
	}
	r := rune(s[0])
	return r >= 'a' && r <= 'z'
}

func joinImportSegments(parts []string) string {
	if len(parts) == 0 {
		return ""
	}
	out := parts[0]
	for _, part := range parts[1:] {
		out += "/" + part
	}
	return out
}

func (p *Parser) parseImportSymbolList() ([]ImportSymbol, Span, error) {
	open := p.previous()
	var symbols []ImportSymbol
	for {
		name, err := p.consume(TokenIdentifier, "expected import symbol")
		if err != nil {
			return nil, Span{}, err
		}
		symbol := ImportSymbol{Name: name.Lexeme, Span: tokenSpan(name)}
		if p.match(TokenAs) {
			alias, err := p.consume(TokenIdentifier, "expected alias after 'as'")
			if err != nil {
				return nil, Span{}, err
			}
			symbol.Alias = alias.Lexeme
			symbol.Span = mergeSpans(symbol.Span, tokenSpan(alias))
		}
		symbols = append(symbols, symbol)
		if !p.match(TokenComma) {
			break
		}
	}
	close, err := p.consume(TokenRBrace, "expected '}' after import symbol list")
	if err != nil {
		return nil, Span{}, err
	}
	return symbols, mergeSpans(tokenSpan(open), tokenSpan(close)), nil
}
