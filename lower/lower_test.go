package lower

import (
	"testing"

	"a-lang/parser"
	"a-lang/typecheck"
	"a-lang/typed"
)

func parseProgram(t *testing.T, src string) *parser.Program {
	t.Helper()
	program, err := parser.Parse(src)
	if err != nil {
		t.Fatalf("Parse returned error: %v", err)
	}
	return program
}

func TestProgramFromTyped(t *testing.T) {
	src := `
class Counter {
	hidden var count Int
}

impl Counter {
	def new(count Int) {
		this.count = count
	}

	def inc() Int {
		this.count += 1
		return this.count
	}
}

seed Int = 1

def run(input Int) Int {
	counter Counter = Counter(input)
	if input > 0 {
		return counter.inc()
	}
	return seed
}
`

	program := parseProgram(t, src)
	types := typecheck.Analyze(program)
	if len(types.Diagnostics) != 0 {
		t.Fatalf("expected no type diagnostics, got %#v", types.Diagnostics)
	}

	typedProgram, err := typed.Build(program, types)
	if err != nil {
		t.Fatalf("typed.Build returned error: %v", err)
	}

	lowered, err := ProgramFromTyped(typedProgram)
	if err != nil {
		t.Fatalf("ProgramFromTyped returned error: %v", err)
	}

	if len(lowered.Globals) != 1 {
		t.Fatalf("expected 1 global, got %d", len(lowered.Globals))
	}
	if len(lowered.Functions) != 1 {
		t.Fatalf("expected 1 function, got %d", len(lowered.Functions))
	}
	if len(lowered.Classes) != 1 {
		t.Fatalf("expected 1 class, got %d", len(lowered.Classes))
	}
	if lowered.Globals[0].Name != "seed" {
		t.Fatalf("unexpected global %#v", lowered.Globals[0])
	}
	if lowered.Classes[0].Constructor == nil {
		t.Fatalf("expected constructor to be lowered")
	}
	if len(lowered.Classes[0].Methods) != 1 || lowered.Classes[0].Methods[0].Name != "inc" {
		t.Fatalf("unexpected lowered methods %#v", lowered.Classes[0].Methods)
	}
	first, ok := lowered.Functions[0].Body[0].(*VarDecl)
	if !ok {
		t.Fatalf("expected first statement to be binding decl, got %T", lowered.Functions[0].Body[0])
	}
	if _, ok := first.Init.(*Construct); !ok {
		t.Fatalf("expected constructor new, got %T", first.Init)
	}
	ifStmt, ok := lowered.Functions[0].Body[1].(*If)
	if !ok {
		t.Fatalf("expected second statement to be if, got %T", lowered.Functions[0].Body[1])
	}
	if _, ok := ifStmt.Then[0].(*Return); !ok {
		t.Fatalf("expected return in then branch, got %T", ifStmt.Then[0])
	}
}

func TestProgramFromTypedLowersLambdaInvokeAndYieldLoop(t *testing.T) {
	src := `
def run(values List[Int]) Int {
	add Int -> Int = x -> x + 1
	for {
		x <- values,
		y <- values
	} yield {
		x + y
	}
	return add(0)
}
`

	program := parseProgram(t, src)
	types := typecheck.Analyze(program)
	if len(types.Diagnostics) != 0 {
		t.Fatalf("expected no type diagnostics, got %#v", types.Diagnostics)
	}
	typedProgram, err := typed.Build(program, types)
	if err != nil {
		t.Fatalf("typed.Build returned error: %v", err)
	}
	lowered, err := ProgramFromTyped(typedProgram)
	if err != nil {
		t.Fatalf("ProgramFromTyped returned error: %v", err)
	}
	first, ok := lowered.Functions[0].Body[0].(*VarDecl)
	if !ok {
		t.Fatalf("expected lambda binding, got %T", lowered.Functions[0].Body[0])
	}
	if _, ok := first.Init.(*Lambda); !ok {
		t.Fatalf("expected lowered lambda, got %T", first.Init)
	}
	second, ok := lowered.Functions[0].Body[1].(*VarDecl)
	if !ok {
		t.Fatalf("expected yield result temp var, got %T", lowered.Functions[0].Body[1])
	}
	if second.Type == nil || second.Type.Name != "List" {
		t.Fatalf("expected yield temp list type, got %#v", second.Type)
	}
	outer, ok := lowered.Functions[0].Body[2].(*ForEach)
	if !ok {
		t.Fatalf("expected outer foreach, got %T", lowered.Functions[0].Body[2])
	}
	inner, ok := outer.Body[0].(*ForEach)
	if !ok {
		t.Fatalf("expected inner foreach, got %T", outer.Body[0])
	}
	assign, ok := inner.Body[0].(*Assign)
	if !ok {
		t.Fatalf("expected append assignment, got %T", inner.Body[0])
	}
	if _, ok := assign.Value.(*BuiltinCall); !ok {
		t.Fatalf("expected append builtin call, got %T", assign.Value)
	}
	ret, ok := lowered.Functions[0].Body[3].(*Return)
	if !ok {
		t.Fatalf("expected return statement, got %T", lowered.Functions[0].Body[3])
	}
	if _, ok := ret.Value.(*Invoke); !ok {
		t.Fatalf("expected lowered invoke, got %T", ret.Value)
	}
}

func TestProgramFromTypedLowersIndexing(t *testing.T) {
	src := `
def run(values Array[Int]) Int {
	return values[0]
}
`

	program := parseProgram(t, src)
	types := typecheck.Analyze(program)
	if len(types.Diagnostics) != 0 {
		t.Fatalf("expected no type diagnostics, got %#v", types.Diagnostics)
	}
	typedProgram, err := typed.Build(program, types)
	if err != nil {
		t.Fatalf("typed.Build returned error: %v", err)
	}

	lowered, err := ProgramFromTyped(typedProgram)
	if err != nil {
		t.Fatalf("ProgramFromTyped returned error: %v", err)
	}

	ret, ok := lowered.Functions[0].Body[0].(*Return)
	if !ok {
		t.Fatalf("expected return statement, got %T", lowered.Functions[0].Body[0])
	}
	if _, ok := ret.Value.(*IndexGet); !ok {
		t.Fatalf("expected lowered index get, got %T", ret.Value)
	}
}

func TestProgramFromTypedLowersIfExpr(t *testing.T) {
	src := `
def choose(flag Bool) Int = if flag {
	1
} else {
	2
}
`

	program := parseProgram(t, src)
	types := typecheck.Analyze(program)
	if len(types.Diagnostics) != 0 {
		t.Fatalf("expected no type diagnostics, got %#v", types.Diagnostics)
	}
	typedProgram, err := typed.Build(program, types)
	if err != nil {
		t.Fatalf("typed.Build returned error: %v", err)
	}

	lowered, err := ProgramFromTyped(typedProgram)
	if err != nil {
		t.Fatalf("ProgramFromTyped returned error: %v", err)
	}

	exprStmt, ok := lowered.Functions[0].Body[0].(*ExprStmt)
	if !ok {
		t.Fatalf("expected expression statement, got %T", lowered.Functions[0].Body[0])
	}
	ifExpr, ok := exprStmt.Expr.(*IfExpr)
	if !ok {
		t.Fatalf("expected lowered if expression, got %T", exprStmt.Expr)
	}
	if len(ifExpr.ThenPrefix) != 0 || len(ifExpr.ElsePrefix) != 0 {
		t.Fatalf("expected branch prefixes to be empty, got then=%d else=%d", len(ifExpr.ThenPrefix), len(ifExpr.ElsePrefix))
	}
}
