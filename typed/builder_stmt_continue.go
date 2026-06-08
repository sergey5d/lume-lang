package typed

import "a-lang/parser"

// continueStmtBuilder builds typed continue statements.
type continueStmtBuilder struct{}

// Build converts a parser continue statement into a typed continue statement.
func (b *continueStmtBuilder) Build(stmt *parser.ContinueStmt) (Stmt, error) {
	return &ContinueStmt{Span: stmt.Span}, nil
}
