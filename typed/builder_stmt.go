package typed

import (
	"fmt"

	"a-lang/parser"
)

// stmtBuilder dispatches parser statements to their dedicated typed builders.
type stmtBuilder struct {
	bindings         Builder[*parser.ValStmt, Stmt]
	localFunctions   Builder[*parser.LocalFunctionStmt, Stmt]
	assignments      Builder[*parser.AssignmentStmt, Stmt]
	multiAssignments Builder[*parser.MultiAssignmentStmt, Stmt]
	ifs              Builder[*parser.IfStmt, Stmt]
	whiles           Builder[*parser.WhileStmt, Stmt]
	fors             Builder[*parser.ForStmt, Stmt]
	returns          Builder[*parser.ReturnStmt, Stmt]
	breaks           Builder[*parser.BreakStmt, Stmt]
	continues        Builder[*parser.ContinueStmt, Stmt]
	exprs            Builder[*parser.ExprStmt, Stmt]
}

// Build converts a parser statement into its typed equivalent.
func (b *stmtBuilder) Build(stmt parser.Statement) (Stmt, error) {
	switch s := stmt.(type) {
	case *parser.ValStmt:
		return b.bindings.Build(s)
	case *parser.LocalFunctionStmt:
		return b.localFunctions.Build(s)
	case *parser.AssignmentStmt:
		return b.assignments.Build(s)
	case *parser.MultiAssignmentStmt:
		return b.multiAssignments.Build(s)
	case *parser.IfStmt:
		return b.ifs.Build(s)
	case *parser.WhileStmt:
		return b.whiles.Build(s)
	case *parser.ForStmt:
		return b.fors.Build(s)
	case *parser.ReturnStmt:
		return b.returns.Build(s)
	case *parser.BreakStmt:
		return b.breaks.Build(s)
	case *parser.ContinueStmt:
		return b.continues.Build(s)
	case *parser.ExprStmt:
		return b.exprs.Build(s)
	default:
		return nil, fmt.Errorf("unsupported statement type %T", stmt)
	}
}
