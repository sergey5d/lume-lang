package parser

import "fmt"

type Lexer struct {
	input  []rune
	pos    int
	line   int
	column int
}

func Lex(input string) ([]Token, error) {
	lexer := &Lexer{
		input:  []rune(input),
		line:   1,
		column: 1,
	}

	var tokens []Token
	for {
		token, err := lexer.nextToken()
		if err != nil {
			return nil, err
		}
		tokens = append(tokens, token)
		if token.Type == TokenEOF {
			return tokens, nil
		}
	}
}

func (l *Lexer) nextToken() (Token, error) {
	l.skipWhitespace()

	startLine, startColumn := l.line, l.column
	if l.isAtEnd() {
		return Token{
			Type:      TokenEOF,
			Line:      startLine,
			Column:    startColumn,
			EndLine:   startLine,
			EndColumn: startColumn,
		}, nil
	}

	ch := l.peek()
	switch ch {
	case '(':
		l.advance()
		return l.token(TokenLParen, "(", startLine, startColumn), nil
	case ')':
		l.advance()
		return l.token(TokenRParen, ")", startLine, startColumn), nil
	case '{':
		l.advance()
		return l.token(TokenLBrace, "{", startLine, startColumn), nil
	case '}':
		l.advance()
		return l.token(TokenRBrace, "}", startLine, startColumn), nil
	case '[':
		l.advance()
		return l.token(TokenLBracket, "[", startLine, startColumn), nil
	case ']':
		l.advance()
		return l.token(TokenRBracket, "]", startLine, startColumn), nil
	case ',':
		l.advance()
		return l.token(TokenComma, ",", startLine, startColumn), nil
	case ':':
		l.advance()
		if l.match('=') {
			return l.token(TokenColonAssign, ":=", startLine, startColumn), nil
		}
		if l.match('+') {
			return l.token(TokenColonPlus, ":+", startLine, startColumn), nil
		}
		if l.match('-') {
			return l.token(TokenColonMinus, ":-", startLine, startColumn), nil
		}
		if l.match(':') {
			return l.token(TokenColonColon, "::", startLine, startColumn), nil
		}
		return l.token(TokenColon, ":", startLine, startColumn), nil
	case '.':
		l.advance()
		if l.match('.') {
			if l.match('.') {
				return l.token(TokenEllipsis, "...", startLine, startColumn), nil
			}
			return Token{}, fmt.Errorf("unexpected token '..' @ %d:%d; use Range(start, end) instead", startLine, startColumn)
		}
		return l.token(TokenDot, ".", startLine, startColumn), nil
	case '?':
		l.advance()
		return l.token(TokenQuestion, "?", startLine, startColumn), nil
	case '@':
		l.advance()
		return l.token(TokenAt, "@", startLine, startColumn), nil
	case '+':
		l.advance()
		if l.match('=') {
			return l.token(TokenPlusEq, "+=", startLine, startColumn), nil
		}
		if l.match('+') {
			return l.token(TokenPlusPlus, "++", startLine, startColumn), nil
		}
		return l.token(TokenPlus, "+", startLine, startColumn), nil
	case '-':
		l.advance()
		if l.match('>') {
			return l.token(TokenArrow, "->", startLine, startColumn), nil
		}
		if l.match('=') {
			return l.token(TokenMinusEq, "-=", startLine, startColumn), nil
		}
		if l.match('-') {
			return l.token(TokenMinusMinus, "--", startLine, startColumn), nil
		}
		return l.token(TokenMinus, "-", startLine, startColumn), nil
	case '*':
		l.advance()
		if l.match('=') {
			return l.token(TokenStarEq, "*=", startLine, startColumn), nil
		}
		return l.token(TokenStar, "*", startLine, startColumn), nil
	case '/':
		l.advance()
		if l.match('=') {
			return l.token(TokenSlashEq, "/=", startLine, startColumn), nil
		}
		return l.token(TokenSlash, "/", startLine, startColumn), nil
	case '%':
		l.advance()
		if l.match('=') {
			return l.token(TokenPercentEq, "%=", startLine, startColumn), nil
		}
		return l.token(TokenPercent, "%", startLine, startColumn), nil
	case '!':
		l.advance()
		if l.match('=') {
			return l.token(TokenBangEq, "!=", startLine, startColumn), nil
		}
		return l.token(TokenBang, "!", startLine, startColumn), nil
	case '_':
		l.advance()
		if !l.isAtEnd() && (isAlphaNumeric(l.peek()) || l.peek() == '_') {
			return l.lexUnderscoreIdentifier(startLine, startColumn), nil
		}
		return l.token(TokenUnder, "_", startLine, startColumn), nil
	case '=':
		l.advance()
		if l.match('=') {
			return l.token(TokenEqEq, "==", startLine, startColumn), nil
		}
		if l.match('>') {
			return l.token(TokenFatArrow, "=>", startLine, startColumn), nil
		}
		return l.token(TokenAssign, "=", startLine, startColumn), nil
	case '<':
		l.advance()
		if l.match('-') {
			return l.token(TokenLeftArrow, "<-", startLine, startColumn), nil
		}
		if l.match('<') {
			return l.token(TokenLTLT, "<<", startLine, startColumn), nil
		}
		if l.match('=') {
			return l.token(TokenLTE, "<=", startLine, startColumn), nil
		}
		return l.token(TokenLT, "<", startLine, startColumn), nil
	case '>':
		l.advance()
		if l.match('>') {
			return l.token(TokenGTGT, ">>", startLine, startColumn), nil
		}
		if l.match('=') {
			return l.token(TokenGTE, ">=", startLine, startColumn), nil
		}
		return l.token(TokenGT, ">", startLine, startColumn), nil
	case '&':
		l.advance()
		if l.match('&') {
			return l.token(TokenAndAnd, "&&", startLine, startColumn), nil
		}
		return l.token(TokenAmp, "&", startLine, startColumn), nil
	case '|':
		l.advance()
		if l.match('|') {
			return l.token(TokenOrOr, "||", startLine, startColumn), nil
		}
		return l.token(TokenPipe, "|", startLine, startColumn), nil
	case '~':
		l.advance()
		return l.token(TokenTilde, "~", startLine, startColumn), nil
	case '\'':
		return l.lexRune(startLine, startColumn)
	case '"':
		return l.lexString(startLine, startColumn)
	}

	if isDigit(ch) {
		return l.lexNumber(startLine, startColumn), nil
	}
	if isAlpha(ch) {
		return l.lexIdentifier(startLine, startColumn), nil
	}

	return Token{}, fmt.Errorf("unexpected %q at %d:%d", ch, startLine, startColumn)
}

func (l *Lexer) lexString(line, column int) (Token, error) {
	if l.pos+2 < len(l.input) && l.input[l.pos] == '"' && l.input[l.pos+1] == '"' && l.input[l.pos+2] == '"' {
		return l.lexMultilineString(line, column)
	}
	l.advance()
	start := l.pos
	for !l.isAtEnd() && l.peek() != '"' {
		if l.peek() == '\n' {
			return Token{}, fmt.Errorf("unterminated string at %d:%d", line, column)
		}
		l.advance()
	}
	if l.isAtEnd() {
		return Token{}, fmt.Errorf("unterminated string at %d:%d", line, column)
	}

	value := string(l.input[start:l.pos])
	l.advance()
	return l.token(TokenString, value, line, column), nil
}

func (l *Lexer) lexMultilineString(line, column int) (Token, error) {
	l.advance()
	l.advance()
	l.advance()
	start := l.pos
	for !l.isAtEnd() {
		if l.peek() == '"' && l.pos+2 < len(l.input) && l.input[l.pos+1] == '"' && l.input[l.pos+2] == '"' {
			value := string(l.input[start:l.pos])
			if value == "" {
				return Token{}, fmt.Errorf("empty multiline string at %d:%d", line, column)
			}
			l.advance()
			l.advance()
			l.advance()
			return l.token(TokenMultilineString, value, line, column), nil
		}
		l.advance()
	}
	return Token{}, fmt.Errorf("unterminated multiline string at %d:%d", line, column)
}

func (l *Lexer) lexRune(line, column int) (Token, error) {
	l.advance()
	if l.isAtEnd() || l.peek() == '\n' {
		return Token{}, fmt.Errorf("unterminated rune literal at %d:%d", line, column)
	}

	var value string
	if l.peek() == '\\' {
		l.advance()
		if l.isAtEnd() || l.peek() == '\n' {
			return Token{}, fmt.Errorf("unterminated rune literal at %d:%d", line, column)
		}
		escape := l.advance()
		switch escape {
		case 'n', 't', 'r', '\\', '\'', '"':
			value = "\\" + string(escape)
		default:
			return Token{}, fmt.Errorf("unsupported char escape \\%c at %d:%d", escape, line, column)
		}
	} else {
		ch := l.advance()
		value = string(ch)
	}

	if l.isAtEnd() || l.peek() != '\'' {
		return Token{}, fmt.Errorf("rune literal must contain exactly one character at %d:%d", line, column)
	}
	l.advance()
	return l.token(TokenRune, value, line, column), nil
}

func (l *Lexer) lexNumber(line, column int) Token {
	start := l.pos
	for !l.isAtEnd() && isDigit(l.peek()) {
		l.advance()
	}
	if !l.isAtEnd() && l.peek() == '.' {
		nextPos := l.pos + 1
		if nextPos >= len(l.input) || l.input[nextPos] != '.' {
			l.advance()
			for !l.isAtEnd() && isDigit(l.peek()) {
				l.advance()
			}
			return l.token(TokenFloat, string(l.input[start:l.pos]), line, column)
		}
	}
	return l.token(TokenInteger, string(l.input[start:l.pos]), line, column)
}

func (l *Lexer) lexIdentifier(line, column int) Token {
	start := l.pos
	for !l.isAtEnd() && isAlphaNumeric(l.peek()) {
		l.advance()
	}
	lexeme := string(l.input[start:l.pos])
	if keyword, ok := keywords[lexeme]; ok {
		return l.token(keyword, lexeme, line, column)
	}
	return l.token(TokenIdentifier, lexeme, line, column)
}

func (l *Lexer) lexUnderscoreIdentifier(line, column int) Token {
	start := l.pos - 1
	for !l.isAtEnd() && (isAlphaNumeric(l.peek()) || l.peek() == '_') {
		l.advance()
	}
	lexeme := string(l.input[start:l.pos])
	return l.token(TokenIdentifier, lexeme, line, column)
}

func (l *Lexer) token(tt TokenType, lexeme string, line, column int) Token {
	return Token{
		Type:      tt,
		Lexeme:    lexeme,
		Line:      line,
		Column:    column,
		EndLine:   l.line,
		EndColumn: l.column,
	}
}

func (l *Lexer) skipWhitespace() {
	for !l.isAtEnd() {
		switch l.peek() {
		case ' ', '\r', '\t':
			l.advance()
		case '\n':
			l.advance()
		case '#':
			l.skipComment()
		default:
			return
		}
	}
}

func (l *Lexer) skipComment() {
	for !l.isAtEnd() && l.peek() != '\n' {
		l.advance()
	}
}

func (l *Lexer) match(expected rune) bool {
	if l.isAtEnd() || l.input[l.pos] != expected {
		return false
	}
	l.advance()
	return true
}

func (l *Lexer) peek() rune {
	return l.input[l.pos]
}

func (l *Lexer) advance() rune {
	ch := l.input[l.pos]
	l.pos++
	if ch == '\n' {
		l.line++
		l.column = 1
	} else {
		l.column++
	}
	return ch
}

func (l *Lexer) isAtEnd() bool {
	return l.pos >= len(l.input)
}

func isDigit(ch rune) bool {
	return ch >= '0' && ch <= '9'
}

func isAlpha(ch rune) bool {
	return (ch >= 'a' && ch <= 'z') || (ch >= 'A' && ch <= 'Z')
}

func isAlphaNumeric(ch rune) bool {
	return isAlpha(ch) || isDigit(ch)
}
