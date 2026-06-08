package parser

import (
	"fmt"
)

type TokenType string

const (
	TokenEOF             TokenType = "EOF"
	TokenIdentifier      TokenType = "IDENT"
	TokenInteger         TokenType = "INT"
	TokenFloat           TokenType = "FLOAT"
	TokenRune            TokenType = "RUNE"
	TokenString          TokenType = "STRING"
	TokenMultilineString TokenType = "MULTILINE_STRING"
	TokenBool            TokenType = "BOOL"

	TokenDef       TokenType = "DEF"
	TokenImpl      TokenType = "IMPL"
	TokenOperator  TokenType = "OPERATOR"
	TokenModule    TokenType = "MODULE"
	TokenImport    TokenType = "IMPORT"
	TokenAs        TokenType = "AS"
	TokenInterface TokenType = "INTERFACE"
	TokenClass     TokenType = "CLASS"
	TokenObject    TokenType = "OBJECT"
	TokenRecord    TokenType = "RECORD"
	TokenEnum      TokenType = "ENUM"
	TokenCase      TokenType = "CASE"
	TokenWith      TokenType = "WITH"
	TokenPrivate   TokenType = "PRIVATE"
	TokenPub       TokenType = "PUB"
	TokenVar       TokenType = "VAR"
	TokenLet       TokenType = "LET"
	TokenIf        TokenType = "IF"
	TokenThen      TokenType = "THEN"
	TokenPartial   TokenType = "PARTIAL"
	TokenMatch     TokenType = "MATCH"
	TokenIs        TokenType = "IS"
	TokenTry       TokenType = "TRY"
	TokenElse      TokenType = "ELSE"
	TokenWhile     TokenType = "WHILE"
	TokenFor       TokenType = "FOR"
	TokenYield     TokenType = "YIELD"
	TokenReturn    TokenType = "RETURN"
	TokenBreak     TokenType = "BREAK"
	TokenContinue  TokenType = "CONTINUE"

	TokenLParen   TokenType = "("
	TokenRParen   TokenType = ")"
	TokenLBrace   TokenType = "{"
	TokenRBrace   TokenType = "}"
	TokenLBracket TokenType = "["
	TokenRBracket TokenType = "]"

	TokenComma       TokenType = ","
	TokenColon       TokenType = ":"
	TokenColonPlus   TokenType = ":+"
	TokenColonMinus  TokenType = ":-"
	TokenColonColon  TokenType = "::"
	TokenDot         TokenType = "."
	TokenQuestion    TokenType = "?"
	TokenAt          TokenType = "@"
	TokenEllipsis    TokenType = "..."
	TokenAssign      TokenType = "="
	TokenFatArrow    TokenType = "=>"
	TokenColonAssign TokenType = ":="

	TokenPlus       TokenType = "+"
	TokenPlusPlus   TokenType = "++"
	TokenMinus      TokenType = "-"
	TokenMinusMinus TokenType = "--"
	TokenStar       TokenType = "*"
	TokenSlash      TokenType = "/"
	TokenPercent    TokenType = "%"

	TokenPlusEq    TokenType = "+="
	TokenMinusEq   TokenType = "-="
	TokenStarEq    TokenType = "*="
	TokenSlashEq   TokenType = "/="
	TokenPercentEq TokenType = "%="

	TokenArrow     TokenType = "->"
	TokenLeftArrow TokenType = "<-"
	TokenEqEq      TokenType = "=="
	TokenBang      TokenType = "!"
	TokenBangEq    TokenType = "!="
	TokenLT        TokenType = "<"
	TokenLTLT      TokenType = "<<"
	TokenLTE       TokenType = "<="
	TokenGT        TokenType = ">"
	TokenGTGT      TokenType = ">>"
	TokenGTE       TokenType = ">="

	TokenAmp    TokenType = "&"
	TokenAndAnd TokenType = "&&"
	TokenPipe   TokenType = "|"
	TokenOrOr   TokenType = "||"
	TokenTilde  TokenType = "~"

	TokenUnder TokenType = "_"
)

var keywords = map[string]TokenType{
	"def":       TokenDef,
	"impl":      TokenImpl,
	"operator":  TokenOperator,
	"module":    TokenModule,
	"import":    TokenImport,
	"as":        TokenAs,
	"interface": TokenInterface,
	"class":     TokenClass,
	"object":    TokenObject,
	"record":    TokenRecord,
	"enum":      TokenEnum,
	"case":      TokenCase,
	"with":      TokenWith,
	"hidden":    TokenPrivate,
	"public":    TokenPub,
	"var":       TokenVar,
	"let":       TokenLet,
	"if":        TokenIf,
	"then":      TokenThen,
	"partial":   TokenPartial,
	"match":     TokenMatch,
	"is":        TokenIs,
	"try":       TokenTry,
	"else":      TokenElse,
	"while":     TokenWhile,
	"for":       TokenFor,
	"yield":     TokenYield,
	"return":    TokenReturn,
	"break":     TokenBreak,
	"continue":  TokenContinue,
	"true":      TokenBool,
	"false":     TokenBool,
}

type Token struct {
	Type      TokenType
	Lexeme    string
	Line      int
	Column    int
	EndLine   int
	EndColumn int
}

func (t Token) String() string {
	return fmt.Sprintf("%s(%q @ %d:%d)", t.Type, t.Lexeme, t.Line, t.Column)
}
