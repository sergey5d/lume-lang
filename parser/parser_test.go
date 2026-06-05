package parser

import (
	"strings"
	"testing"
)

const sampleProgram = `
def doSomeWork(a Int, b Int) Bool {

	list = [a, b, c]
	set = Set()
	map = Map()
	map2 = Map(a : b)
	tuple = Set(a, b)
	tuple2 = a : b : c
	tuple21 = a : b : c
	array = Array(1, 2, 3)
	string Str = "xxx"
	a int = 65

	a Int, b Int64 = 1, 3

	if a == b {

	} else {

	}

	for a <- list {

		if a == 7 {
			break
		}
	}

	for a <- Range(1, 100) {

	}

	for {
		a <- list,
		c <- map
	} yield {
		a + c
	}

	return a == 5
}
`

func assertTypeRef(t *testing.T, ref *TypeRef, name string, argNames ...string) {
	t.Helper()
	if ref == nil {
		t.Fatalf("expected type %q, got nil", name)
	}
	if ref.Name != name {
		t.Fatalf("expected type %q, got %q", name, ref.Name)
	}
	if len(ref.Arguments) != len(argNames) {
		t.Fatalf("expected %d type arguments for %q, got %d", len(argNames), name, len(ref.Arguments))
	}
	for i, argName := range argNames {
		if ref.Arguments[i].Name != argName {
			t.Fatalf("expected type argument %d for %q to be %q, got %q", i, name, argName, ref.Arguments[i].Name)
		}
	}
}

func TestParseSampleProgram(t *testing.T) {
	program, err := Parse(sampleProgram)
	if err != nil {
		t.Fatalf("Parse returned error: %v", err)
	}
	if len(program.Functions) != 1 {
		t.Fatalf("expected 1 function, got %d", len(program.Functions))
	}
	fn := program.Functions[0]
	if fn.Name != "doSomeWork" {
		t.Fatalf("unexpected function name %q", fn.Name)
	}
	if got := len(fn.Body.Statements); got != 16 {
		t.Fatalf("expected 16 statements in body, got %d", got)
	}
	if _, ok := fn.Body.Statements[len(fn.Body.Statements)-1].(*ReturnStmt); !ok {
		t.Fatalf("expected final statement to be return, got %T", fn.Body.Statements[len(fn.Body.Statements)-1])
	}
}

func TestParseMatchStmt(t *testing.T) {
	program, err := Parse(`
def run(value OptionX[Int]) Int {
	match value {
	case SomeX(x) => {
			return x
		}
	case OptionX.NoneX => {
			return 0
		}
	}
}

enum OptionX[T] {
	case NoneX
	case SomeX {
		value T
	}
}
`)
	if err != nil {
		t.Fatalf("Parse returned error: %v", err)
	}
	fn := program.Functions[0]
	matchStmt, ok := fn.Body.Statements[0].(*MatchStmt)
	if !ok {
		t.Fatalf("expected match statement, got %T", fn.Body.Statements[0])
	}
	if _, ok := matchStmt.Cases[0].Pattern.(*ConstructorPattern); !ok {
		t.Fatalf("expected constructor pattern, got %T", matchStmt.Cases[0].Pattern)
	}
	if _, ok := matchStmt.Cases[1].Pattern.(*ConstructorPattern); !ok {
		t.Fatalf("expected qualified constructor pattern, got %T", matchStmt.Cases[1].Pattern)
	}
}

func TestParseMatchGuard(t *testing.T) {
	program, err := Parse(`
enum OptionX[T] {
	case NoneX
	case SomeX {
		value T
	}
}

def run(value OptionX[Int]) Int =
	match value {
	case SomeX(x) if x > 10 => x
	case _ => 0
	}
`)
	if err != nil {
		t.Fatalf("Parse returned error: %v", err)
	}
	fn := program.Functions[0]
	exprStmt, ok := fn.Body.Statements[0].(*ExprStmt)
	if !ok {
		t.Fatalf("expected expression statement, got %T", fn.Body.Statements[0])
	}
	matchExpr, ok := exprStmt.Expr.(*MatchExpr)
	if !ok {
		t.Fatalf("expected match expression, got %T", exprStmt.Expr)
	}
	if matchExpr.Cases[0].Guard == nil {
		t.Fatalf("expected first case to have guard")
	}
}

func TestParseMatchTypePattern(t *testing.T) {
	program, err := Parse(`
class Worker {
}

def run(value Worker) Int {
	match value {
	case item Worker => {
			return 1
		}
	case _ Worker => {
			return 2
		}
	}
}
`)
	if err != nil {
		t.Fatalf("Parse returned error: %v", err)
	}
	fn := program.Functions[0]
	matchStmt := fn.Body.Statements[0].(*MatchStmt)
	if _, ok := matchStmt.Cases[0].Pattern.(*TypePattern); !ok {
		t.Fatalf("expected type pattern, got %T", matchStmt.Cases[0].Pattern)
	}
	if _, ok := matchStmt.Cases[1].Pattern.(*TypePattern); !ok {
		t.Fatalf("expected wildcard type pattern, got %T", matchStmt.Cases[1].Pattern)
	}
}

func TestParseNestedMatchPatterns(t *testing.T) {
	program, err := Parse(`
enum AppleBox {
	case Empty
	case Full {
		value (Int, Int)
	}
}

def run(value AppleBox) Int =
	match value {
	case Full((left, right)) => left + right
	case AppleBox.Empty => 0
	}
`)
	if err != nil {
		t.Fatalf("Parse returned error: %v", err)
	}
	fn := program.Functions[0]
	exprStmt := fn.Body.Statements[0].(*ExprStmt)
	matchExpr := exprStmt.Expr.(*MatchExpr)
	constructor, ok := matchExpr.Cases[0].Pattern.(*ConstructorPattern)
	if !ok {
		t.Fatalf("expected constructor pattern, got %T", matchExpr.Cases[0].Pattern)
	}
	if _, ok := constructor.Args[0].(*TuplePattern); !ok {
		t.Fatalf("expected nested tuple pattern, got %T", constructor.Args[0])
	}
}

func TestParseRejectsDeepNestedMatchPatterns(t *testing.T) {
	_, err := Parse(`
enum OptionX[T] {
	case NoneX
	case SomeX {
		value T
	}
}

class Box {
	value Int
}

def run(value OptionX[(Box, Int)]) Int =
	match value {
	case SomeX((Box(x), y)) => x + y
	case OptionX.NoneX => 0
	}
`)
	if err == nil {
		t.Fatalf("expected parse error for deep nested pattern")
	}
}

func TestParseRejectsNestedTypePatterns(t *testing.T) {
	_, err := Parse(`
class Worker {
}

enum OptionX[T] {
	case NoneX
	case SomeX {
		value T
	}
}

def run(value OptionX[Worker]) Int =
	match value {
		case SomeX(item Worker) => 1
		case OptionX.NoneX => 0
	}
`)
	if err == nil {
		t.Fatalf("expected parse error for nested type pattern")
	}
}

func TestParseStringInterpolation(t *testing.T) {
	program, err := Parse(`
def run(name Str, count Int) Str {
	return "hello $name ${count + 1} \\$done"
}
`)
	if err != nil {
		t.Fatalf("Parse returned error: %v", err)
	}
	fn := program.Functions[0]
	ret, ok := fn.Body.Statements[0].(*ReturnStmt)
	if !ok {
		t.Fatalf("expected return statement, got %T", fn.Body.Statements[0])
	}
	if _, ok := ret.Value.(*BinaryExpr); !ok {
		t.Fatalf("expected interpolated string to lower to BinaryExpr, got %T", ret.Value)
	}
}

func TestParseMultilineString(t *testing.T) {
	program, err := Parse(`
def run() Str {
	return """
hello
$name
\n
"""
}
`)
	if err != nil {
		t.Fatalf("Parse returned error: %v", err)
	}
	fn := program.Functions[0]
	ret := fn.Body.Statements[0].(*ReturnStmt)
	literal, ok := ret.Value.(*StringLiteral)
	if !ok {
		t.Fatalf("expected multiline string to stay a StringLiteral, got %T", ret.Value)
	}
	if literal.Value != "\nhello\n$name\n\n\n" {
		t.Fatalf("unexpected multiline string value %q", literal.Value)
	}
}

func TestParseRejectsEmptyMultilineString(t *testing.T) {
	_, err := Parse(`
def run() Str {
	return """"""
}
`)
	if err == nil {
		t.Fatalf("expected parse error for empty multiline string")
	}
}

func TestParseNestedBlockExpressions(t *testing.T) {
	program, err := Parse(`
def run() Int {
	a1 = {
		1 + 7
	}
	{
		OS.println("xxx")
	}
	var v = {
		a = 5
		{
			a + 1
		}
	}
	return a1 + v
}
`)
	if err != nil {
		t.Fatalf("Parse returned error: %v", err)
	}
	fn := program.Functions[0]
	first, ok := fn.Body.Statements[0].(*ValStmt)
	if !ok {
		t.Fatalf("expected first statement to be ValStmt, got %T", fn.Body.Statements[0])
	}
	if _, ok := first.Values[0].(*BlockExpr); !ok {
		t.Fatalf("expected first binding value to be BlockExpr, got %T", first.Values[0])
	}
	second, ok := fn.Body.Statements[1].(*ExprStmt)
	if !ok {
		t.Fatalf("expected second statement to be ExprStmt, got %T", fn.Body.Statements[1])
	}
	if _, ok := second.Expr.(*BlockExpr); !ok {
		t.Fatalf("expected standalone block statement to be BlockExpr, got %T", second.Expr)
	}
	third, ok := fn.Body.Statements[2].(*ValStmt)
	if !ok {
		t.Fatalf("expected third statement to be ValStmt, got %T", fn.Body.Statements[2])
	}
	if !third.Bindings[0].Mutable {
		t.Fatalf("expected third binding to be mutable")
	}
	outer, ok := third.Values[0].(*BlockExpr)
	if !ok {
		t.Fatalf("expected mutable binding value to be BlockExpr, got %T", third.Values[0])
	}
	last, ok := outer.Body.Statements[len(outer.Body.Statements)-1].(*ExprStmt)
	if !ok {
		t.Fatalf("expected nested block to be final ExprStmt, got %T", outer.Body.Statements[len(outer.Body.Statements)-1])
	}
	if _, ok := last.Expr.(*BlockExpr); !ok {
		t.Fatalf("expected nested block expr, got %T", last.Expr)
	}
}

func TestParseHashComments(t *testing.T) {
	src := `
# top-level comment
def run() Int {
	# before binding
	value Int = 1 # trailing comment
	# before return
	return value
}
`

	program, err := Parse(src)
	if err != nil {
		t.Fatalf("Parse returned error: %v", err)
	}
	if len(program.Functions) != 1 {
		t.Fatalf("expected 1 function, got %d", len(program.Functions))
	}
	fn := program.Functions[0]
	if got := len(fn.Body.Statements); got != 2 {
		t.Fatalf("expected 2 statements in body, got %d", got)
	}
	if _, ok := fn.Body.Statements[0].(*ValStmt); !ok {
		t.Fatalf("expected first statement to be binding, got %T", fn.Body.Statements[0])
	}
	if _, ok := fn.Body.Statements[1].(*ReturnStmt); !ok {
		t.Fatalf("expected second statement to be return, got %T", fn.Body.Statements[1])
	}
}

func TestParseForForms(t *testing.T) {
	src := `
def loops(input Int, value Int) Bool {
	for item <- [1, 2, 3] {
		if item == input {
			break
		}
	}

	for item <- [1, 3] {
		if item == input {
			break
		}
	}

	while true {
		break
	}

	while value < 3 {
		break
	}

	for left Int, right Str <- [(1, "x")] {
		if left == value {
			break
		}
	}

	for {
		x <- [value],
		y <- [input],
		sum = x + y
	} yield {
		sum
	}

	return value == input
}
`

	program, err := Parse(src)
	if err != nil {
		t.Fatalf("Parse returned error: %v", err)
	}

	fn := program.Functions[0]
	first := fn.Body.Statements[0].(*ForStmt)
	if len(first.Bindings) != 1 || first.Body == nil {
		t.Fatalf("expected single-binding for loop, got %#v", first)
	}

	third := fn.Body.Statements[2].(*WhileStmt)
	if third.Condition == nil || third.Body == nil {
		t.Fatalf("expected infinite while loop, got %#v", third)
	}

	fourth := fn.Body.Statements[3].(*WhileStmt)
	if fourth.Condition == nil || fourth.Body == nil {
		t.Fatalf("expected while loop, got %#v", fourth)
	}

	fifth := fn.Body.Statements[4].(*ForStmt)
	if len(fifth.Bindings) != 1 || len(fifth.Bindings[0].Bindings) != 2 || fifth.Body == nil {
		t.Fatalf("expected destructuring for loop, got %#v", fifth)
	}

	sixth := fn.Body.Statements[5].(*ForStmt)
	if len(sixth.Bindings) != 3 || sixth.YieldBody == nil {
		t.Fatalf("expected yield-style for loop, got %#v", sixth)
	}
	if len(sixth.Bindings[2].Values) != 1 {
		t.Fatalf("expected third yield clause to be immutable local binding, got %#v", sixth.Bindings[2])
	}
}

func TestParseIfAndForYieldExpressions(t *testing.T) {
	src := `
def run(values List[Int], flag Bool) Int {
	label = if flag {
		1
	} else {
		2
	}

	items = for {
		x <- values,
		y <- values
	} yield {
		x + y
	}

	items2 = for item <- values yield {
		item + 1
	}

	return label
}
`

	program, err := Parse(src)
	if err != nil {
		t.Fatalf("Parse returned error: %v", err)
	}

	fn := program.Functions[0]
	first := fn.Body.Statements[0].(*ValStmt)
	if _, ok := first.Values[0].(*IfExpr); !ok {
		t.Fatalf("expected first binding value to be if expression, got %T", first.Values[0])
	}
	second := fn.Body.Statements[1].(*ValStmt)
	if _, ok := second.Values[0].(*ForYieldExpr); !ok {
		t.Fatalf("expected second binding value to be yield expression, got %T", second.Values[0])
	}
	third := fn.Body.Statements[2].(*ValStmt)
	if yieldExpr, ok := third.Values[0].(*ForYieldExpr); !ok {
		t.Fatalf("expected third binding value to be short yield expression, got %T", third.Values[0])
	} else if len(yieldExpr.Bindings) != 1 {
		t.Fatalf("expected short yield expression to have 1 binding, got %d", len(yieldExpr.Bindings))
	}
}

func TestParseShortStatements(t *testing.T) {
	src := `
def run(values List[Int], flag Bool, maybe MaybeInt) Int {
	if flag then return 1 else return 2
	for value <- values {
		OS.println(value)
	}
	while true {
		break
	}
	match maybe {
	case SomeX(x) => {
			return x
		}
	}
}

enum MaybeInt {
	case NoneX
	case SomeX {
		value Int
	}
}
`

	program, err := Parse(src)
	if err != nil {
		t.Fatalf("Parse returned error: %v", err)
	}

	fn := program.Functions[0]
	ifStmt, ok := fn.Body.Statements[0].(*IfStmt)
	if !ok {
		t.Fatalf("expected first statement to be if, got %T", fn.Body.Statements[0])
	}
	if len(ifStmt.Then.Statements) != 1 || len(ifStmt.Else.Statements) != 1 {
		t.Fatalf("expected shorthand if to wrap single statements, got %#v", ifStmt)
	}

	forStmt, ok := fn.Body.Statements[1].(*ForStmt)
	if !ok {
		t.Fatalf("expected second statement to be for, got %T", fn.Body.Statements[1])
	}
	if len(forStmt.Body.Statements) != 1 {
		t.Fatalf("expected shorthand for body to wrap one statement, got %#v", forStmt.Body)
	}

	whileStmt, ok := fn.Body.Statements[2].(*WhileStmt)
	if !ok {
		t.Fatalf("expected third statement to be while, got %T", fn.Body.Statements[2])
	}
	if len(whileStmt.Body.Statements) != 1 {
		t.Fatalf("expected while body to wrap one statement, got %#v", whileStmt.Body)
	}

	matchStmt, ok := fn.Body.Statements[3].(*MatchStmt)
	if !ok {
		t.Fatalf("expected fourth statement to be match, got %T", fn.Body.Statements[3])
	}
	if len(matchStmt.Cases) != 1 {
		t.Fatalf("expected 1 match case, got %d", len(matchStmt.Cases))
	}
	if matchStmt.Cases[0].Body == nil {
		t.Fatalf("expected match statement case to have body, got %#v", matchStmt.Cases)
	}
}

func TestParseColonShorthandExpressions(t *testing.T) {
	src := `
def run(values List[Int], flag Bool, maybe MaybeInt) Int {
	label = if flag then 1 else 2
	label2 = if flag then 3
	else 4
	label3 = if flag then 5
	else if false then 6
	else 7
	items = for value <- values yield value + 1
	picked = match maybe {
	case SomeX(x) => x
	case MaybeInt.NoneX => 0
	}
	return label + picked
}

enum MaybeInt {
	case NoneX
	case SomeX {
		value Int
	}
}
`

	program, err := Parse(src)
	if err != nil {
		t.Fatalf("Parse returned error: %v", err)
	}

	fn := program.Functions[0]
	first := fn.Body.Statements[0].(*ValStmt)
	ifExpr, ok := first.Values[0].(*IfExpr)
	if !ok {
		t.Fatalf("expected first binding value to be if expression, got %T", first.Values[0])
	}
	if len(ifExpr.Then.Statements) != 1 || len(ifExpr.Else.Statements) != 1 {
		t.Fatalf("expected shorthand if expression blocks to wrap single expressions, got %#v", ifExpr)
	}

	secondIf := fn.Body.Statements[1].(*ValStmt)
	ifExpr2, ok := secondIf.Values[0].(*IfExpr)
	if !ok {
		t.Fatalf("expected second binding value to be multiline shorthand if expression, got %T", secondIf.Values[0])
	}
	if len(ifExpr2.Then.Statements) != 1 || len(ifExpr2.Else.Statements) != 1 {
		t.Fatalf("expected multiline shorthand if expression blocks to wrap single expressions, got %#v", ifExpr2)
	}

	thirdIf := fn.Body.Statements[2].(*ValStmt)
	ifExpr3, ok := thirdIf.Values[0].(*IfExpr)
	if !ok {
		t.Fatalf("expected third binding value to be shorthand else-if expression, got %T", thirdIf.Values[0])
	}
	if nestedElse, ok := ifExpr3.Else.Statements[0].(*ExprStmt); !ok {
		t.Fatalf("expected else-if shorthand to be wrapped as expression statement, got %#v", ifExpr3.Else.Statements)
	} else if _, ok := nestedElse.Expr.(*IfExpr); !ok {
		t.Fatalf("expected else branch to contain nested if expression, got %T", nestedElse.Expr)
	}

	fourth := fn.Body.Statements[3].(*ValStmt)
	forYield, ok := fourth.Values[0].(*ForYieldExpr)
	if !ok {
		t.Fatalf("expected fourth binding value to be for-yield expression, got %T", fourth.Values[0])
	}
	if len(forYield.YieldBody.Statements) != 1 {
		t.Fatalf("expected shorthand yield body to wrap one expression, got %#v", forYield.YieldBody)
	}

	fifth := fn.Body.Statements[4].(*ValStmt)
	matchExpr, ok := fifth.Values[0].(*MatchExpr)
	if !ok {
		t.Fatalf("expected fifth binding value to be match expression, got %T", fifth.Values[0])
	}
	if len(matchExpr.Cases) != 2 {
		t.Fatalf("expected 2 match expression cases, got %d", len(matchExpr.Cases))
	}
	if matchExpr.Cases[0].Expr == nil || matchExpr.Cases[1].Expr == nil {
		t.Fatalf("expected match expression cases to have expressions, got %#v", matchExpr.Cases)
	}
	if matchExpr.Partial {
		t.Fatalf("expected plain match expression to be exhaustive by default")
	}
}

func TestParsePartialMatchExpr(t *testing.T) {
	src := `
def run(value MaybeInt) Option[Int] =
	partial value {
	case SomeX(x) => x + 1
	}
`

	program, err := Parse(src)
	if err != nil {
		t.Fatalf("Parse returned error: %v", err)
	}

	fn := program.Functions[0]
	stmt, ok := fn.Body.Statements[0].(*ExprStmt)
	if !ok {
		t.Fatalf("expected expression statement, got %T", fn.Body.Statements[0])
	}
	expr, ok := stmt.Expr.(*MatchExpr)
	if !ok {
		t.Fatalf("expected match expression, got %T", stmt.Expr)
	}
	if !expr.Partial {
		t.Fatalf("expected partial match expression")
	}
}

func TestParsePartialMethodInImpl(t *testing.T) {
	src := `
class Classifier {
}

impl Classifier {
	partial classify(value Int) Int {
	case 3 => 5
	case 4 => 0
	}
}
`

	program, err := Parse(src)
	if err != nil {
		t.Fatalf("Parse returned error: %v", err)
	}
	method := program.Classes[0].Methods[0]
	if !method.Partial {
		t.Fatalf("expected method to be partial")
	}
	if method.ReturnType == nil || method.ReturnType.Name != "Option" || len(method.ReturnType.Arguments) != 1 || method.ReturnType.Arguments[0].Name != "Int" {
		t.Fatalf("expected partial method return type to be Option[Int], got %#v", method.ReturnType)
	}
}

func TestParseThenShorthandRejectsNewlineBody(t *testing.T) {
	src := `
def run(flag Bool) Int {
	if flag then
		return 1
	return 0
}
`

	if _, err := Parse(src); err == nil {
		t.Fatalf("expected parse error for newline body after then")
	}
}

func TestParseThenShorthandAllowsMultilineRecordLiteral(t *testing.T) {
	src := `
def run(flag Bool) { value Int } {
	if flag then return record {
		value = 1
	}
	return record { value = 0 }
}
`

	if _, err := Parse(src); err != nil {
		t.Fatalf("Parse returned error: %v", err)
	}
}

func TestParseNewlineContinuationAfterOperatorsAndDelimiters(t *testing.T) {
	src := `
def run() Int {
	sum Int = 1 +
		2
	grouped Int = (
		3
		+ 4
	)
	items List[Int] = [
		1,
		2,
		3
	]
	size Int = "haha".
		size()
	return sum + grouped + items.size() + size
}
`

	if _, err := Parse(src); err != nil {
		t.Fatalf("Parse returned error: %v", err)
	}
}

func TestParseAssignmentRejectsNextLineRHS(t *testing.T) {
	src := `
def run() Int {
	value =
		1 + 2
	return value
}
`

	if _, err := Parse(src); err == nil {
		t.Fatalf("expected parse error for newline rhs after '='")
	}
}

func TestParseForRejectsOneLineBody(t *testing.T) {
	src := `
def run(values List[Int]) Unit {
	for value <- values OS.println(value)
}
`

	_, err := Parse(src)
	if err == nil {
		t.Fatalf("expected parse error for one-line for body")
	}
	if got := err.Error(); got != "for requires a '{ ... }' block body; one-line for forms are not supported" {
		t.Fatalf("unexpected parse error: %v", err)
	}
}

func TestParseForRejectsConditionLoop(t *testing.T) {
	src := `
def run(limit Int) Int {
	for limit < 3 {
		return limit
	}
	return 0
}
`

	_, err := Parse(src)
	if err == nil {
		t.Fatalf("expected parse error for conditional for loop")
	}
	if got := err.Error(); got != "for requires '<-' bindings or 'yield'; use 'while' for condition loops" {
		t.Fatalf("unexpected parse error: %v", err)
	}
}

func TestParseIsExpression(t *testing.T) {
	src := `
def run(value Any) Bool {
	return value is Str
}
`

	program, err := Parse(src)
	if err != nil {
		t.Fatalf("Parse returned error: %v", err)
	}

	fn := program.Functions[0]
	ret := fn.Body.Statements[0].(*ReturnStmt)
	isExpr, ok := ret.Value.(*IsExpr)
	if !ok {
		t.Fatalf("expected return value to be is expression, got %T", ret.Value)
	}
	if isExpr.Target == nil || isExpr.Target.Name != "Str" {
		t.Fatalf("expected is target Str, got %#v", isExpr.Target)
	}
}

func TestParseModuleAndImports(t *testing.T) {
	src := `
	module app
	import util
	import model/user
	
	def main() Unit {
		()
	}
	`

	program, err := Parse(src)
	if err != nil {
		t.Fatalf("Parse returned error: %v", err)
	}
	if program.ModuleName != "app" {
		t.Fatalf("expected module app, got %q", program.ModuleName)
	}
	if len(program.Imports) != 2 {
		t.Fatalf("expected 2 imports, got %d", len(program.Imports))
	}
	if program.Imports[0].Path != "util" || program.Imports[1].Path != "model/user" {
		t.Fatalf("unexpected imports %#v", program.Imports)
	}
}

func TestParseExtendedImportForms(t *testing.T) {
	src := `
	import model/things
	import model/things/*
	import model/things/A
	import model/things/A as B
	import model/things/{A, B as D, C}
	import model/things/C/*
	import model/things/C/{printLn as printN, print}
`

	program, err := Parse(src)
	if err != nil {
		t.Fatalf("Parse returned error: %v", err)
	}
	if len(program.Imports) != 7 {
		t.Fatalf("expected 7 imports, got %d", len(program.Imports))
	}
	if program.Imports[0].Path != "model/things" || program.Imports[0].Wildcard || len(program.Imports[0].Symbols) != 0 {
		t.Fatalf("unexpected module import %#v", program.Imports[0])
	}
	if !program.Imports[1].Wildcard || program.Imports[1].Path != "model/things" {
		t.Fatalf("unexpected wildcard import %#v", program.Imports[1])
	}
	if got := program.Imports[2].Symbols; len(got) != 1 || got[0].Name != "A" || got[0].Alias != "" {
		t.Fatalf("unexpected single import %#v", program.Imports[2])
	}
	if got := program.Imports[3].Symbols; len(got) != 1 || got[0].Name != "A" || got[0].Alias != "B" {
		t.Fatalf("unexpected aliased import %#v", program.Imports[3])
	}
	if got := program.Imports[4].Symbols; len(got) != 3 || got[0].Name != "A" || got[1].Name != "B" || got[1].Alias != "D" || got[2].Name != "C" {
		t.Fatalf("unexpected grouped import %#v", program.Imports[4])
	}
	if program.Imports[5].Path != "model/things" || program.Imports[5].ObjectName != "C" || !program.Imports[5].Wildcard {
		t.Fatalf("unexpected object wildcard import %#v", program.Imports[5])
	}
	if got := program.Imports[6].Symbols; program.Imports[6].Path != "model/things" || program.Imports[6].ObjectName != "C" || len(got) != 2 || got[0].Name != "printLn" || got[0].Alias != "printN" || got[1].Name != "print" {
		t.Fatalf("unexpected object member import %#v", program.Imports[6])
	}
}

func TestParseAnnotations(t *testing.T) {
	src := `
record Tag {
	name Str
}

record Route {
	path Str
}

	@Tag(name = "service")
	class Service {
		@Tag(name = "field")
		name Str
	}

	impl Service {
		@Route(path = "/health")
		def health() Str = "ok"
	}
`

	program, err := Parse(src)
	if err != nil {
		t.Fatalf("Parse returned error: %v", err)
	}
	if len(program.Classes) < 3 {
		t.Fatalf("expected declarations, got %#v", program.Classes)
	}
	service := program.Classes[2]
	if service.Name != "Service" || len(service.Annotations) != 1 {
		t.Fatalf("expected annotated Service class, got %#v", service)
	}
	if len(service.Fields) != 1 || len(service.Fields[0].Annotations) != 1 {
		t.Fatalf("expected annotated field, got %#v", service.Fields)
	}
	if len(service.Methods) != 1 || len(service.Methods[0].Annotations) != 1 {
		t.Fatalf("expected annotated impl method, got %#v", service.Methods)
	}
}

func TestParseMethodWithoutReturnType(t *testing.T) {
	src := `
class Counter {
}

impl Counter {
	def touch() {
		1
	}
}
`

	program, err := Parse(src)
	if err != nil {
		t.Fatalf("Parse returned error: %v", err)
	}
	method := program.Classes[0].Methods[0]
	if method.Constructor {
		t.Fatalf("expected ordinary method, got constructor %#v", method)
	}
	if method.ReturnType == nil || method.ReturnType.Name != "Unit" {
		t.Fatalf("expected omitted return type to normalize to Unit, got %#v", method.ReturnType)
	}
}

func TestParseThisConstructorAndFieldOrder(t *testing.T) {
	src := `
class Counter {
	count Int
}

impl Counter {
	def init(seed Int) {
		init(count = seed)
	}
}
`

	program, err := Parse(src)
	if err != nil {
		t.Fatalf("Parse returned error: %v", err)
	}
	method := program.Classes[0].Methods[0]
	if !method.Constructor || method.Name != "init" {
		t.Fatalf("expected def init to be marked as constructor, got %#v", method)
	}

	bad := `
class Broken {
	count Int
}

impl Broken {
	def run() Unit {
	}
}
`
	if _, err := Parse(bad); err != nil {
		t.Fatalf("expected impl form to parse, got %v", err)
	}
}

func TestParseExpressionBodiedDefs(t *testing.T) {
	src := `
def suffix(value Str) Str = value + "!"

class Counter {
}

impl Counter {
	def value() Int = 1
}
`

	program, err := Parse(src)
	if err != nil {
		t.Fatalf("Parse returned error: %v", err)
	}

	fn := program.Functions[0]
	if len(fn.Body.Statements) != 1 {
		t.Fatalf("expected 1 statement in function body, got %d", len(fn.Body.Statements))
	}
	if _, ok := fn.Body.Statements[0].(*ExprStmt); !ok {
		t.Fatalf("expected expression-bodied function to parse as ExprStmt block, got %T", fn.Body.Statements[0])
	}

	method := program.Classes[0].Methods[0]
	if len(method.Body.Statements) != 1 {
		t.Fatalf("expected 1 statement in method body, got %d", len(method.Body.Statements))
	}
	if _, ok := method.Body.Statements[0].(*ExprStmt); !ok {
		t.Fatalf("expected expression-bodied method to parse as ExprStmt block, got %T", method.Body.Statements[0])
	}
}

func TestParseExplicitUnitDefs(t *testing.T) {
	src := `
def printIt() Unit = OS.println("x")

class Counter {
}

impl Counter {
	def print() Unit = OS.println("y")
}
`

	program, err := Parse(src)
	if err != nil {
		t.Fatalf("Parse returned error: %v", err)
	}

	if program.Functions[0].ReturnType == nil || program.Functions[0].ReturnType.Name != "Unit" {
		t.Fatalf("expected Unit return type for function, got %#v", program.Functions[0].ReturnType)
	}
	method := program.Classes[0].Methods[0]
	if method.ReturnType == nil || method.ReturnType.Name != "Unit" {
		t.Fatalf("expected Unit return type for method, got %#v", method.ReturnType)
	}
}

func TestParseFunctionWithoutReturnType(t *testing.T) {
	src := `
def printIt() {
	1
}
`

	program, err := Parse(src)
	if err != nil {
		t.Fatalf("Parse returned error: %v", err)
	}

	fn := program.Functions[0]
	if fn.ReturnType == nil || fn.ReturnType.Name != "Unit" {
		t.Fatalf("expected Unit return type, got %#v", fn.ReturnType)
	}
}

func TestParseLocalFunctionStmt(t *testing.T) {
	src := `
def run() Int {
	def x(term Int) = term + 1
	x(5)
}
`

	program, err := Parse(src)
	if err != nil {
		t.Fatalf("Parse returned error: %v", err)
	}

	fn := program.Functions[0]
	if len(fn.Body.Statements) != 2 {
		t.Fatalf("expected 2 statements, got %d", len(fn.Body.Statements))
	}
	if _, ok := fn.Body.Statements[0].(*LocalFunctionStmt); !ok {
		t.Fatalf("expected first statement to be local function, got %T", fn.Body.Statements[0])
	}
}

func TestParseEqualsBlockBodiedDefs(t *testing.T) {
	src := `
def top() = {
	1
}

class Counter {
}

impl Counter {
	def method() = {
		1
	}
}

def run() {
	def local() = {
		1
	}
	local()
}
`

	program, err := Parse(src)
	if err != nil {
		t.Fatalf("Parse returned error: %v", err)
	}

	if len(program.Functions[0].Body.Statements) != 1 {
		t.Fatalf("expected top function block body to contain 1 statement, got %d", len(program.Functions[0].Body.Statements))
	}
	if len(program.Classes[0].Methods[0].Body.Statements) != 1 {
		t.Fatalf("expected method block body to contain 1 statement, got %d", len(program.Classes[0].Methods[0].Body.Statements))
	}
	run := program.Functions[1]
	localStmt, ok := run.Body.Statements[0].(*LocalFunctionStmt)
	if !ok {
		t.Fatalf("expected first run statement to be local function, got %T", run.Body.Statements[0])
	}
	if len(localStmt.Function.Body.Statements) != 1 {
		t.Fatalf("expected local function block body to contain 1 statement, got %d", len(localStmt.Function.Body.Statements))
	}
}

func TestParseVariadicParameter(t *testing.T) {
	src := `
def printAll(values Str...) {
	values.size()
}
`

	program, err := Parse(src)
	if err != nil {
		t.Fatalf("Parse returned error: %v", err)
	}

	param := program.Functions[0].Parameters[0]
	if !param.Variadic {
		t.Fatalf("expected parameter to be variadic")
	}
}

func TestParseMultiAssignmentStmt(t *testing.T) {
	src := `
def run() Int {
	var a Int = 0
	var b Int = 0
	a, b := 1, 2
	a
}
`

	program, err := Parse(src)
	if err != nil {
		t.Fatalf("Parse returned error: %v", err)
	}

	fn := program.Functions[0]
	if _, ok := fn.Body.Statements[2].(*MultiAssignmentStmt); !ok {
		t.Fatalf("expected multi assignment statement, got %T", fn.Body.Statements[2])
	}
}

func TestParseDeclaredNamesWithEqualsAsBindings(t *testing.T) {
	src := `
def run() Int {
	var a Int = 0
	var b Int = 0
	a, b = 1, 2
	a
}
`

	program, err := Parse(src)
	if err != nil {
		t.Fatalf("Parse returned error: %v", err)
	}

	fn := program.Functions[0]
	if _, ok := fn.Body.Statements[2].(*ValStmt); !ok {
		t.Fatalf("expected binding statement, got %T", fn.Body.Statements[2])
	}
}

func TestParseTupleLiteralAndType(t *testing.T) {
	src := `
def run() (Int, Str) {
	pair (Int, Str) = (1, "ok")
	pair
}
`

	program, err := Parse(src)
	if err != nil {
		t.Fatalf("Parse returned error: %v", err)
	}

	fn := program.Functions[0]
	if fn.ReturnType == nil || len(fn.ReturnType.TupleElements) != 2 {
		t.Fatalf("expected tuple return type, got %#v", fn.ReturnType)
	}
	stmt := fn.Body.Statements[0].(*ValStmt)
	if stmt.Bindings[0].Type == nil || len(stmt.Bindings[0].Type.TupleElements) != 2 {
		t.Fatalf("expected tuple binding type, got %#v", stmt.Bindings[0].Type)
	}
	if _, ok := stmt.Values[0].(*TupleLiteral); !ok {
		t.Fatalf("expected tuple literal, got %T", stmt.Values[0])
	}
}

func TestParseUnitAndGroupedParenExpr(t *testing.T) {
	src := `
def run() Unit {
	unit = ()
	value = (1)
	pair = (1, 2)
}
`

	program, err := Parse(src)
	if err != nil {
		t.Fatalf("Parse returned error: %v", err)
	}

	body := program.Functions[0].Body.Statements
	unitStmt := body[0].(*ValStmt)
	if _, ok := unitStmt.Values[0].(*UnitLiteral); !ok {
		t.Fatalf("expected unit literal, got %T", unitStmt.Values[0])
	}

	groupStmt := body[1].(*ValStmt)
	if _, ok := groupStmt.Values[0].(*GroupExpr); !ok {
		t.Fatalf("expected grouped expression, got %T", groupStmt.Values[0])
	}

	tupleStmt := body[2].(*ValStmt)
	if _, ok := tupleStmt.Values[0].(*TupleLiteral); !ok {
		t.Fatalf("expected tuple literal, got %T", tupleStmt.Values[0])
	}
}

func TestParseZeroArgFunctionBindingSugar(t *testing.T) {
	src := `
def run() Unit {
	action () -> Unit = OS.println("x")
}
`

	program, err := Parse(src)
	if err != nil {
		t.Fatalf("Parse returned error: %v", err)
	}

	stmt := program.Functions[0].Body.Statements[0].(*ValStmt)
	if _, ok := stmt.Values[0].(*LambdaExpr); !ok {
		t.Fatalf("expected lambda expression, got %T", stmt.Values[0])
	}
}

func TestParseUnitFunctionType(t *testing.T) {
	src := `
def run(action () -> Unit) Unit {
	action()
}
`

	program, err := Parse(src)
	if err != nil {
		t.Fatalf("Parse returned error: %v", err)
	}

	fnType := program.Functions[0].Parameters[0].Type
	if fnType.ReturnType == nil || fnType.ReturnType.Name != "Unit" {
		t.Fatalf("expected Unit return type, got %#v", fnType)
	}
	if len(fnType.ParameterTypes) != 0 {
		t.Fatalf("expected zero-arg function type, got %#v", fnType.ParameterTypes)
	}
}

func TestParseExtendedOperators(t *testing.T) {
	src := `
def ops(a Int, b Int) Bool {
	x = a - b / 2 * 3 % 5
	y = !a == b
	z = a != b
	c = a < b
	d = a <= b
	e = a > b
	f = a >= b
	yes = true
	no = false
	pi = 1.1
	whole = 1.
	return a == b || a != b && !(a < b)
}
`

	program, err := Parse(src)
	if err != nil {
		t.Fatalf("Parse returned error: %v", err)
	}
	if len(program.Functions) != 1 {
		t.Fatalf("expected 1 function, got %d", len(program.Functions))
	}
}

func TestParseBoolAndFloatLiterals(t *testing.T) {
	src := `
def literals() Bool {
	yes = true
	no = false
	a = 1.1
	b = 1.
	c = 'x'
	d = '\n'
	return yes == true
}
`

	program, err := Parse(src)
	if err != nil {
		t.Fatalf("Parse returned error: %v", err)
	}

	fn := program.Functions[0]
	if _, ok := fn.Body.Statements[0].(*ValStmt).Values[0].(*BoolLiteral); !ok {
		t.Fatalf("expected true to parse as bool literal")
	}
	if _, ok := fn.Body.Statements[1].(*ValStmt).Values[0].(*BoolLiteral); !ok {
		t.Fatalf("expected false to parse as bool literal")
	}
	if _, ok := fn.Body.Statements[2].(*ValStmt).Values[0].(*FloatLiteral); !ok {
		t.Fatalf("expected 1.1 to parse as float literal")
	}
	if literal, ok := fn.Body.Statements[3].(*ValStmt).Values[0].(*FloatLiteral); !ok || literal.Value != "1." {
		t.Fatalf("expected 1. to parse as float literal, got %#v", fn.Body.Statements[3].(*ValStmt).Values[0])
	}
	if literal, ok := fn.Body.Statements[4].(*ValStmt).Values[0].(*RuneLiteral); !ok || literal.Value != "x" {
		t.Fatalf("expected 'x' to parse as rune literal, got %#v", fn.Body.Statements[4].(*ValStmt).Values[0])
	}
	if literal, ok := fn.Body.Statements[5].(*ValStmt).Values[0].(*RuneLiteral); !ok || literal.Value != "\\n" {
		t.Fatalf("expected '\\n' to parse as escaped rune literal, got %#v", fn.Body.Statements[5].(*ValStmt).Values[0])
	}
}

func TestParseInterfacesAndClasses(t *testing.T) {
	src := `
interface Mapper[K, V] {
	def map(key K) V
}

interface Stringable {
	def show() Str
}

class Box[T] with Mapper[T, Stringable] {
	hidden value T
}

impl Box[T] {
	def init(value T) {
		this.value = value
	}

	def map(key T) Stringable {
		return this
	}
}

class SolidWork with Stringable {
	hidden a List[Int]
	hidden var b Map[Str, Bool]
}

impl SolidWork {
	def init(a Int, b Bool) {
		this.a = a
		this.b = b
	}

	def show() Str {
	}

	def addOne(one Int) Int {
	}

	hidden def buildLabel() Str {
	}
}

class RecordKeeper {
	entries Set[Str]
	hidden approved Bool
}

recordKeeper = RecordKeeper("test record", true)
solidWork = SolidWork(1, false)
`

	program, err := Parse(src)
	if err != nil {
		t.Fatalf("Parse returned error: %v", err)
	}
	if len(program.Interfaces) != 2 {
		t.Fatalf("expected 2 interfaces, got %d", len(program.Interfaces))
	}
	if len(program.Classes) != 3 {
		t.Fatalf("expected 3 classes, got %d", len(program.Classes))
	}
	if len(program.Statements) != 2 {
		t.Fatalf("expected 2 top-level statements, got %d", len(program.Statements))
	}
	mapper := program.Interfaces[0]
	if mapper.Name != "Mapper" || len(mapper.TypeParameters) != 2 {
		t.Fatalf("unexpected generic interface %#v", mapper)
	}
	assertTypeRef(t, mapper.Methods[0].Parameters[0].Type, "K")
	assertTypeRef(t, mapper.Methods[0].ReturnType, "V")

	box := program.Classes[0]
	if box.Name != "Box" || len(box.TypeParameters) != 1 {
		t.Fatalf("unexpected generic class %#v", box)
	}
	if len(box.Implements) != 1 || box.Implements[0].Name != "Mapper" {
		t.Fatalf("unexpected generic implements clause %#v", box.Implements)
	}
	assertTypeRef(t, box.Implements[0], "Mapper", "T", "Stringable")
	assertTypeRef(t, box.Fields[0].Type, "T")

	cls := program.Classes[1]
	if cls.Name != "SolidWork" || len(cls.Implements) != 1 || cls.Implements[0].Name != "Stringable" {
		t.Fatalf("unexpected class declaration %#v", cls)
	}
	if len(cls.Fields) != 2 || len(cls.Methods) != 4 {
		t.Fatalf("unexpected class members %#v", cls)
	}
	assertTypeRef(t, cls.Fields[0].Type, "List", "Int")
	assertTypeRef(t, cls.Fields[1].Type, "Map", "Str", "Bool")
	if !cls.Fields[0].Private || !cls.Fields[1].Private {
		t.Fatalf("expected first class fields to be private")
	}
	if !cls.Methods[3].Private {
		t.Fatalf("expected helper method to be private")
	}
	if !cls.Methods[0].Constructor {
		t.Fatalf("expected this to be marked as constructor")
	}
}

func TestParseRecordDecl(t *testing.T) {
	src := `
record Amount {
	amount Int
	description Str
}

impl Amount {
	def label() Str = description
}
`

	program, err := Parse(src)
	if err != nil {
		t.Fatalf("Parse returned error: %v", err)
	}
	if len(program.Classes) != 1 {
		t.Fatalf("expected 1 aggregate decl, got %d", len(program.Classes))
	}
	if !program.Classes[0].Record {
		t.Fatalf("expected declaration to be marked as record")
	}
}

func TestParseObjectDecl(t *testing.T) {
	src := `
object A {
	def value() Unit = ()
	def test(a Int) Int = a + 5
}
`

	program, err := Parse(src)
	if err != nil {
		t.Fatalf("Parse returned error: %v", err)
	}
	if len(program.Classes) != 1 {
		t.Fatalf("expected 1 declaration, got %d", len(program.Classes))
	}
	if !program.Classes[0].Object {
		t.Fatalf("expected declaration to be marked as object")
	}
	if len(program.Classes[0].Methods) != 2 {
		t.Fatalf("expected 2 object methods, got %#v", program.Classes[0].Methods)
	}
}

func TestParseGenericFunctionAndMethodDecl(t *testing.T) {
	src := `
def id[T](value T) T = value

class Mapper {
}

impl Mapper {
	def map[X](value Int, fn Int -> X) X {
		fn(value)
	}
}
`

	program, err := Parse(src)
	if err != nil {
		t.Fatalf("Parse returned error: %v", err)
	}
	if len(program.Functions) != 1 || len(program.Functions[0].TypeParameters) != 1 {
		t.Fatalf("expected generic function declaration, got %#v", program.Functions)
	}
	if len(program.Classes) != 1 || len(program.Classes[0].Methods) != 1 {
		t.Fatalf("expected class with one method, got %#v", program.Classes)
	}
	method := program.Classes[0].Methods[0]
	if len(method.TypeParameters) != 1 {
		t.Fatalf("expected generic method declaration, got %#v", method)
	}
	if method.Parameters[1].Type == nil || method.Parameters[1].Type.ReturnType == nil {
		t.Fatalf("expected function-typed parameter, got %#v", method.Parameters[1].Type)
	}
}

func TestParseGenericBounds(t *testing.T) {
	src := `
def sort[T with Ordering[T]](value T) T = value

class Box[T with Ordering[T]] {
	value T
}

impl Box[T] {
	def map[X with Eq[X]](fn T -> X) X = fn(value)
}
`

	program, err := Parse(src)
	if err != nil {
		t.Fatalf("Parse returned error: %v", err)
	}
	if len(program.Functions) != 1 || len(program.Functions[0].TypeParameters) != 1 {
		t.Fatalf("expected bounded generic function, got %#v", program.Functions)
	}
	if len(program.Functions[0].TypeParameters[0].Bounds) != 1 {
		t.Fatalf("expected function bound, got %#v", program.Functions[0].TypeParameters[0])
	}
	assertTypeRef(t, program.Functions[0].TypeParameters[0].Bounds[0], "Ordering", "T")
	if len(program.Classes) != 1 || len(program.Classes[0].TypeParameters) != 1 {
		t.Fatalf("expected bounded generic class, got %#v", program.Classes)
	}
	assertTypeRef(t, program.Classes[0].TypeParameters[0].Bounds[0], "Ordering", "T")
	method := program.Classes[0].Methods[0]
	if len(method.TypeParameters) != 1 || len(method.TypeParameters[0].Bounds) != 1 {
		t.Fatalf("expected bounded generic method, got %#v", method)
	}
	assertTypeRef(t, method.TypeParameters[0].Bounds[0], "Eq", "X")
}

func TestParseObjectShortApplyDeclRejected(t *testing.T) {
	src := `
object Range {
	def (end Int) Int = end
}
`

	if _, err := Parse(src); err == nil {
		t.Fatalf("expected parse error for short object apply declaration")
	}
}

func TestParseOperatorDecls(t *testing.T) {
	src := `
interface Addable[T] {
	def +(other T) T
}

class Vec {
}

impl Vec {
	def [](index Int) Int = 0
	def +(other Vec) Vec = this
	def -() Vec = this
	def :+(value Int) Vec = this
	def :-(value Int) Vec = this
	def ++(other Vec) Vec = this
	def --(other Vec) Vec = this
	def |(other Vec) Vec = this
	def &(other Vec) Vec = this
	def >>(bits Int) Vec = this
	def <<(bits Int) Vec = this
	def ~() Vec = this
	def ::(other Vec) Vec = this
}
`

	program, err := Parse(src)
	if err != nil {
		t.Fatalf("Parse returned error: %v", err)
	}
	if len(program.Interfaces) != 1 || program.Interfaces[0].Methods[0].Name != "+" {
		t.Fatalf("expected interface operator method, got %#v", program.Interfaces)
	}
	if len(program.Classes) != 1 || len(program.Classes[0].Methods) != 13 {
		t.Fatalf("unexpected operator declarations %#v", program.Classes)
	}
	if !program.Classes[0].Methods[0].Operator || program.Classes[0].Methods[0].Name != "[]" {
		t.Fatalf("expected [] operator method, got %#v", program.Classes[0].Methods[0])
	}
	if !program.Classes[0].Methods[2].Operator || program.Classes[0].Methods[2].Name != "-" {
		t.Fatalf("expected unary - operator method, got %#v", program.Classes[0].Methods[2])
	}
}

func TestParseIncrementDecrementFormsRejected(t *testing.T) {
	for _, src := range []string{
		"count++",
		"count--",
		"++count",
		"--count",
	} {
		if _, err := ParseExpr(src); err == nil {
			t.Fatalf("expected parse error for %q", src)
		}
	}
}

func TestParsePrivateClassDecl(t *testing.T) {
	src := `
hidden class Hidden {
}

impl Hidden {
	def value() Int = 1
}
`

	program, err := Parse(src)
	if err != nil {
		t.Fatalf("Parse returned error: %v", err)
	}
	if len(program.Classes) != 1 {
		t.Fatalf("expected 1 declaration, got %d", len(program.Classes))
	}
	if !program.Classes[0].Private {
		t.Fatalf("expected class to be marked private")
	}
}

func TestParsePrivateTopLevelDecls(t *testing.T) {
	src := `
hidden def helper() Int = 1

hidden interface Hidden {
	def value() Int
}
`

	program, err := Parse(src)
	if err != nil {
		t.Fatalf("Parse returned error: %v", err)
	}
	if len(program.Functions) != 1 || !program.Functions[0].Private {
		t.Fatalf("expected hidden function, got %#v", program.Functions)
	}
	if len(program.Interfaces) != 1 || !program.Interfaces[0].Private {
		t.Fatalf("expected hidden interface, got %#v", program.Interfaces)
	}
}

func TestParsePublicTopLevelDecls(t *testing.T) {
	src := `
public def helper() Int = 1
public answer Int = 42
`

	program, err := Parse(src)
	if err != nil {
		t.Fatalf("Parse returned error: %v", err)
	}
	if len(program.Functions) != 1 || !program.Functions[0].Public {
		t.Fatalf("expected public function, got %#v", program.Functions)
	}
	if len(program.Statements) != 1 {
		t.Fatalf("expected 1 top-level statement, got %d", len(program.Statements))
	}
	stmt, ok := program.Statements[0].(*ValStmt)
	if !ok || !stmt.Public {
		t.Fatalf("expected public top-level binding, got %#v", program.Statements[0])
	}
}

func TestParseDestructuringSkipBinding(t *testing.T) {
	src := `
def run() Int {
	let (a Int, _, c Str) = (1, 2, "x")
	return a
}
`

	program, err := Parse(src)
	if err != nil {
		t.Fatalf("Parse returned error: %v", err)
	}
	stmt, ok := program.Functions[0].Body.Statements[0].(*ValStmt)
	if !ok {
		t.Fatalf("expected binding statement, got %#v", program.Functions[0].Body.Statements[0])
	}
	if len(stmt.Bindings) != 3 || stmt.Bindings[1].Name != "_" {
		t.Fatalf("expected skip binding in middle, got %#v", stmt.Bindings)
	}
}

func TestParseImplicitDestructuringRequiresLet(t *testing.T) {
	src := `
def run() Unit {
	pair = (1, "x")
	left, right = pair
}
`

	_, err := Parse(src)
	if err == nil {
		t.Fatalf("expected parse error, got nil")
	}
	if !strings.Contains(err.Error(), "destructuring bindings require 'let (...) = value'") {
		t.Fatalf("unexpected error: %v", err)
	}
}

func TestParseStatementBoundaryBeforeTypedBinding(t *testing.T) {
	src := `
def run() Unit {
	xxx
	lambda6 = (left Int, right Int) -> left + right
}
`

	program, err := Parse(src)
	if err != nil {
		t.Fatalf("Parse returned error: %v", err)
	}
	fn := program.Functions[0]
	if len(fn.Body.Statements) != 2 {
		t.Fatalf("expected 2 statements, got %d", len(fn.Body.Statements))
	}
	if _, ok := fn.Body.Statements[0].(*ExprStmt); !ok {
		t.Fatalf("expected first statement to stay an expression statement, got %T", fn.Body.Statements[0])
	}
	if _, ok := fn.Body.Statements[1].(*ValStmt); !ok {
		t.Fatalf("expected second statement to be a binding, got %T", fn.Body.Statements[1])
	}
}

func TestParseInterfaceInheritance(t *testing.T) {
	src := `
interface Hopper {
	def hop() Str
}
interface Acrobat with Hopper {
	def land() Str
}
`

	program, err := Parse(src)
	if err != nil {
		t.Fatalf("Parse returned error: %v", err)
	}
	if len(program.Interfaces) != 2 {
		t.Fatalf("expected 2 interfaces, got %d", len(program.Interfaces))
	}
	acrobat := program.Interfaces[1]
	if acrobat.Name != "Acrobat" {
		t.Fatalf("unexpected interface %#v", acrobat)
	}
	if len(acrobat.Extends) != 1 || acrobat.Extends[0].Name != "Hopper" {
		t.Fatalf("unexpected extends clause %#v", acrobat.Extends)
	}
}

func TestParseInterfaceDefaultMethod(t *testing.T) {
	src := `
interface Hopper {
	def hop() Str = "hop"
}
`

	program, err := Parse(src)
	if err != nil {
		t.Fatalf("Parse returned error: %v", err)
	}
	if len(program.Interfaces) != 1 {
		t.Fatalf("expected 1 interface, got %d", len(program.Interfaces))
	}
	method := program.Interfaces[0].Methods[0]
	if method.Body == nil {
		t.Fatalf("expected interface method body")
	}
	if method.ReturnType == nil || method.ReturnType.Name != "Str" {
		t.Fatalf("expected Str return type, got %#v", method.ReturnType)
	}
}

func TestParseRecordUpdateExpr(t *testing.T) {
	src := `
record Amount {
	amount Int
	description Str
}

def run() Amount {
	value = Amount(10, "x")
	return value with { amount = 42, description = "y" }
}
`

	program, err := Parse(src)
	if err != nil {
		t.Fatalf("Parse returned error: %v", err)
	}
	run := program.Functions[0]
	ret, ok := run.Body.Statements[1].(*ReturnStmt)
	if !ok {
		t.Fatalf("expected return statement, got %#v", run.Body.Statements[1])
	}
	update, ok := ret.Value.(*RecordUpdateExpr)
	if !ok {
		t.Fatalf("expected record update expr, got %#v", ret.Value)
	}
	if len(update.Updates) != 2 || update.Updates[0].Name != "amount" || update.Updates[1].Name != "description" {
		t.Fatalf("unexpected record updates %#v", update.Updates)
	}
}

func TestParseAnonymousInterfaceExpr(t *testing.T) {
	src := `
interface Reader {
	def read() Str
}

interface Closer {
	def close() Unit
}

def run() Str {
	handler = Reader with Closer {
		def read() Str = "x"
		def close() Unit = ()
	}
	return handler.read()
}
`

	program, err := Parse(src)
	if err != nil {
		t.Fatalf("Parse returned error: %v", err)
	}
	run := program.Functions[0]
	binding, ok := run.Body.Statements[0].(*ValStmt)
	if !ok {
		t.Fatalf("expected binding statement, got %#v", run.Body.Statements[0])
	}
	anon, ok := binding.Values[0].(*AnonymousInterfaceExpr)
	if !ok {
		t.Fatalf("expected anonymous interface expr, got %#v", binding.Values[0])
	}
	if len(anon.Interfaces) != 2 || anon.Interfaces[0].Name != "Reader" || anon.Interfaces[1].Name != "Closer" {
		t.Fatalf("unexpected interfaces %#v", anon.Interfaces)
	}
	if len(anon.Methods) != 2 || anon.Methods[0].Name != "read" || anon.Methods[1].Name != "close" {
		t.Fatalf("unexpected methods %#v", anon.Methods)
	}
}

func TestParseAnonymousRecordExpr(t *testing.T) {
	src := `
def run(user { name Str, age Int }) Int {
	value = record {
		name = "Ana"
		age = 10
		city = "NYC"
	}
	return user.age
}
`

	program, err := Parse(src)
	if err != nil {
		t.Fatalf("Parse returned error: %v", err)
	}
	run := program.Functions[0]
	if len(run.Parameters) != 1 || len(run.Parameters[0].Type.RecordFields) != 2 {
		t.Fatalf("expected anonymous record parameter type, got %#v", run.Parameters)
	}
	binding, ok := run.Body.Statements[0].(*ValStmt)
	if !ok {
		t.Fatalf("expected binding statement, got %#v", run.Body.Statements[0])
	}
	record, ok := binding.Values[0].(*AnonymousRecordExpr)
	if !ok {
		t.Fatalf("expected anonymous record expr, got %#v", binding.Values[0])
	}
	if len(record.Fields) != 3 || record.Fields[0].Name != "name" || record.Fields[1].Name != "age" || record.Fields[2].Name != "city" {
		t.Fatalf("unexpected anonymous record fields %#v", record.Fields)
	}

	mixedExpr, err := ParseExpr(`record { a = 5, c = 7,
		b = 8
	}`)
	if err != nil {
		t.Fatalf("ParseExpr returned error for mixed record literal: %v", err)
	}
	mixed, ok := mixedExpr.(*AnonymousRecordExpr)
	if !ok {
		t.Fatalf("expected mixed literal to be AnonymousRecordExpr, got %#v", mixedExpr)
	}
	if len(mixed.Fields) != 3 || mixed.Fields[0].Name != "a" || mixed.Fields[1].Name != "c" || mixed.Fields[2].Name != "b" {
		t.Fatalf("unexpected mixed record fields %#v", mixed.Fields)
	}

	identPositionalExpr, err := ParseExpr(`record { name, age }`)
	if err != nil {
		t.Fatalf("ParseExpr returned error for identifier positional record literal: %v", err)
	}
	identPositional, ok := identPositionalExpr.(*AnonymousRecordExpr)
	if !ok {
		t.Fatalf("expected identifier positional literal to be AnonymousRecordExpr, got %#v", identPositionalExpr)
	}
	if len(identPositional.Values) != 2 || len(identPositional.Fields) != 0 {
		t.Fatalf("unexpected identifier positional record literal %#v", identPositional)
	}

	if _, err := ParseExpr(`record(1, "x")`); err == nil {
		t.Fatalf("expected record(...) to be rejected")
	}

	bracePositionalExpr, err := ParseExpr(`record { 1, "x" }`)
	if err != nil {
		t.Fatalf("ParseExpr returned error for brace positional record literal: %v", err)
	}
	bracePositional, ok := bracePositionalExpr.(*AnonymousRecordExpr)
	if !ok {
		t.Fatalf("expected brace positional literal to be AnonymousRecordExpr, got %#v", bracePositionalExpr)
	}
	if len(bracePositional.Values) != 2 || len(bracePositional.Fields) != 0 {
		t.Fatalf("unexpected brace positional record literal %#v", bracePositional)
	}

	trailingProgram, err := Parse(`
def run() Int {
	name = "Ada"
	age = 10
	person = Person { name, age }
	return 0
}
`)
	if err != nil {
		t.Fatalf("Parse returned error for trailing brace record call sugar: %v", err)
	}
	personBinding, ok := trailingProgram.Functions[0].Body.Statements[2].(*ValStmt)
	if !ok {
		t.Fatalf("expected trailing brace record call binding, got %#v", trailingProgram.Functions[0].Body.Statements[2])
	}
	call, ok := personBinding.Values[0].(*CallExpr)
	if !ok || len(call.Args) != 1 {
		t.Fatalf("expected trailing brace record call to be CallExpr, got %#v", personBinding.Values[0])
	}
	recordArg, ok := call.Args[0].Value.(*AnonymousRecordExpr)
	if !ok || len(recordArg.Values) != 2 || len(recordArg.Fields) != 0 {
		t.Fatalf("unexpected trailing brace record call arg %#v", call.Args[0].Value)
	}
}

func TestRejectBareBraceRecordCallArgInsideParens(t *testing.T) {
	if _, err := ParseExpr(`MixedProfile({ name = "Liam", age = 8 })`); err == nil {
		t.Fatalf("expected parse error for bare brace record argument inside call parentheses")
	} else if got := err.Error(); got != "bare '{ ... }' record arguments are not allowed inside '(...)'; use 'Type { ... }' or 'Type(record { ... })'" {
		t.Fatalf("unexpected parse error: %v", err)
	}
}

func TestRejectInvalidRuneLiterals(t *testing.T) {
	cases := []string{
		"def bad() Bool { a = '' return true }",
		"def bad() Bool { a = 'ab' return true }",
	}

	for _, src := range cases {
		if _, err := Parse(src); err == nil {
			t.Fatalf("expected parse error for invalid rune literal in %q", src)
		}
	}
}

func TestParseMutableBindings(t *testing.T) {
	src := `
def vars() Bool {
	var count Int = 1
	var left Int, right Int = 1, 2
	count := count + 1
	return count == right
}
`

	program, err := Parse(src)
	if err != nil {
		t.Fatalf("Parse returned error: %v", err)
	}

	fn := program.Functions[0]
	first, ok := fn.Body.Statements[0].(*ValStmt)
	if !ok {
		t.Fatalf("expected first statement to be binding, got %T", fn.Body.Statements[0])
	}
	if !first.Bindings[0].Mutable {
		t.Fatalf("expected first binding to be mutable")
	}

	second, ok := fn.Body.Statements[1].(*ValStmt)
	if !ok {
		t.Fatalf("expected second statement to be binding, got %T", fn.Body.Statements[1])
	}
	if !second.Bindings[0].Mutable {
		t.Fatalf("expected first binding in second statement to be mutable")
	}
	if !second.Bindings[1].Mutable {
		t.Fatalf("expected second binding in second statement to be mutable")
	}
}

func TestParseAssignmentStatement(t *testing.T) {
	src := `
def vars() Bool {
	var counter = 0
	counter := counter + 1
	counter += 2
	counter -= 1
	counter *= 3
	counter /= 2
	counter %= 2
	return counter == 1
}
`

	program, err := Parse(src)
	if err != nil {
		t.Fatalf("Parse returned error: %v", err)
	}

	fn := program.Functions[0]
	assign, ok := fn.Body.Statements[1].(*AssignmentStmt)
	if !ok {
		t.Fatalf("expected second statement to be assignment, got %T", fn.Body.Statements[1])
	}
	if _, ok := assign.Target.(*Identifier); !ok {
		t.Fatalf("expected assignment target to be identifier, got %T", assign.Target)
	}
	if assign.Operator != ":=" {
		t.Fatalf("expected mutable assignment operator, got %q", assign.Operator)
	}

	compound, ok := fn.Body.Statements[2].(*AssignmentStmt)
	if !ok || compound.Operator != "+=" {
		t.Fatalf("expected compound assignment with +=, got %#v", fn.Body.Statements[2])
	}
}

func TestParseIndexExpressionAndAssignment(t *testing.T) {
	src := `
def run(values Array[Int]) Int {
	item Int = values[0]
	values[1] := item
	return values[1]
}
`

	program, err := Parse(src)
	if err != nil {
		t.Fatalf("Parse returned error: %v", err)
	}

	fn := program.Functions[0]
	first, ok := fn.Body.Statements[0].(*ValStmt)
	if !ok {
		t.Fatalf("expected first statement to be binding, got %T", fn.Body.Statements[0])
	}
	index, ok := first.Values[0].(*IndexExpr)
	if !ok {
		t.Fatalf("expected binding value to be index expression, got %T", first.Values[0])
	}
	if _, ok := index.Receiver.(*Identifier); !ok {
		t.Fatalf("expected index receiver to be identifier, got %T", index.Receiver)
	}

	assign, ok := fn.Body.Statements[1].(*AssignmentStmt)
	if !ok {
		t.Fatalf("expected second statement to be assignment, got %T", fn.Body.Statements[1])
	}
	if _, ok := assign.Target.(*IndexExpr); !ok {
		t.Fatalf("expected assignment target to be index expression, got %T", assign.Target)
	}
	if assign.Operator != ":=" {
		t.Fatalf("expected mutable assignment operator, got %q", assign.Operator)
	}
}

func TestParseImmutableBindings(t *testing.T) {
	src := `
def vars() Bool {
	count Int = 1
	left Int, right Int = 1, 2
	return count == right
}
`

	program, err := Parse(src)
	if err != nil {
		t.Fatalf("Parse returned error: %v", err)
	}

	fn := program.Functions[0]
	first := fn.Body.Statements[0].(*ValStmt)
	if first.Bindings[0].Mutable {
		t.Fatalf("expected immutable binding")
	}

	second := fn.Body.Statements[1].(*ValStmt)
	if second.Bindings[0].Mutable || second.Bindings[1].Mutable {
		t.Fatalf("expected immutable bindings")
	}
}

func TestParseUntypedBindings(t *testing.T) {
	src := `
def vars() Bool {
	a = "some string"
	var counter = 0
	return counter == 0
}
`

	program, err := Parse(src)
	if err != nil {
		t.Fatalf("Parse returned error: %v", err)
	}

	fn := program.Functions[0]

	letStmt := fn.Body.Statements[0].(*ValStmt)
	if letStmt.Bindings[0].Type != nil {
		t.Fatalf("expected untyped immutable binding, got type %#v", letStmt.Bindings[0].Type)
	}
	if letStmt.Bindings[0].Mutable {
		t.Fatalf("expected immutable binding")
	}

	mutableStmt := fn.Body.Statements[1].(*ValStmt)
	if mutableStmt.Bindings[0].Type != nil {
		t.Fatalf("expected untyped mutable binding, got type %#v", mutableStmt.Bindings[0].Type)
	}
	if !mutableStmt.Bindings[0].Mutable {
		t.Fatalf("expected mutable binding to be mutable")
	}
}

func TestParseFunctionInvocationBinding(t *testing.T) {
	src := `
def vars(b Int) Bool {
	a = function(b)
	return a == b
}
`

	program, err := Parse(src)
	if err != nil {
		t.Fatalf("Parse returned error: %v", err)
	}

	fn := program.Functions[0]
	stmt := fn.Body.Statements[0].(*ValStmt)
	call, ok := stmt.Values[0].(*CallExpr)
	if !ok {
		t.Fatalf("expected binding value to be call expression, got %T", stmt.Values[0])
	}
	callee, ok := call.Callee.(*Identifier)
	if !ok || callee.Name != "function" {
		t.Fatalf("expected callee to be identifier 'function', got %#v", call.Callee)
	}
	if len(call.Args) != 1 {
		t.Fatalf("expected 1 call argument, got %d", len(call.Args))
	}
}

func TestParseNamedCallArguments(t *testing.T) {
	src := `
def doSomething(a Str, b Int) Unit {
}

def main() Unit {
	doSomething(a = "crap", b = 5)
}
`

	program, err := Parse(src)
	if err != nil {
		t.Fatalf("Parse returned error: %v", err)
	}

	stmt := program.Functions[1].Body.Statements[0].(*ExprStmt)
	call := stmt.Expr.(*CallExpr)
	if len(call.Args) != 2 {
		t.Fatalf("expected 2 call arguments, got %d", len(call.Args))
	}
	if call.Args[0].Name != "a" || call.Args[1].Name != "b" {
		t.Fatalf("expected named arguments a, b, got %#v", call.Args)
	}
}

func TestParseLambdaBindings(t *testing.T) {
	src := `
def vars() Bool {
	a = Map(1 : "string").map((key, value) -> key.show() + value)
	b = Set(1).map(key -> key.show())
	c = Map(1 : 2).map((key Int, value Int) -> key + value)
	d = Set(1).map(key Int -> key.show())
	return 1 == 1
}
`

	program, err := Parse(src)
	if err != nil {
		t.Fatalf("Parse returned error: %v", err)
	}

	fn := program.Functions[0]

	first := fn.Body.Statements[0].(*ValStmt)
	firstCall, ok := first.Values[0].(*CallExpr)
	if !ok {
		t.Fatalf("expected first binding value to be call expression, got %T", first.Values[0])
	}
	firstLambda, ok := firstCall.Args[0].Value.(*LambdaExpr)
	if !ok {
		t.Fatalf("expected map argument to be lambda, got %T", firstCall.Args[0])
	}
	if len(firstLambda.Parameters) != 2 {
		t.Fatalf("expected 2 lambda parameters, got %d", len(firstLambda.Parameters))
	}

	second := fn.Body.Statements[1].(*ValStmt)
	secondCall, ok := second.Values[0].(*CallExpr)
	if !ok {
		t.Fatalf("expected second binding value to be call expression, got %T", second.Values[0])
	}
	secondLambda, ok := secondCall.Args[0].Value.(*LambdaExpr)
	if !ok {
		t.Fatalf("expected map argument to be lambda, got %T", secondCall.Args[0])
	}
	if len(secondLambda.Parameters) != 1 || secondLambda.Parameters[0].Name != "key" || secondLambda.Parameters[0].Type != nil {
		t.Fatalf("unexpected single-parameter lambda: %#v", secondLambda.Parameters)
	}

	third := fn.Body.Statements[2].(*ValStmt)
	thirdCall, ok := third.Values[0].(*CallExpr)
	if !ok {
		t.Fatalf("expected third binding value to be call expression, got %T", third.Values[0])
	}
	thirdLambda, ok := thirdCall.Args[0].Value.(*LambdaExpr)
	if !ok {
		t.Fatalf("expected typed map argument to be lambda, got %T", thirdCall.Args[0])
	}
	if thirdLambda.Parameters[0].Type == nil || thirdLambda.Parameters[0].Type.Name != "Int" || thirdLambda.Parameters[1].Type == nil || thirdLambda.Parameters[1].Type.Name != "Int" {
		t.Fatalf("expected typed lambda parameters, got %#v", thirdLambda.Parameters)
	}

	fourth := fn.Body.Statements[3].(*ValStmt)
	fourthCall, ok := fourth.Values[0].(*CallExpr)
	if !ok {
		t.Fatalf("expected fourth binding value to be call expression, got %T", fourth.Values[0])
	}
	fourthLambda, ok := fourthCall.Args[0].Value.(*LambdaExpr)
	if !ok {
		t.Fatalf("expected typed single-parameter lambda, got %T", fourthCall.Args[0])
	}
	if len(fourthLambda.Parameters) != 1 || fourthLambda.Parameters[0].Name != "key" || fourthLambda.Parameters[0].Type == nil || fourthLambda.Parameters[0].Type.Name != "Int" {
		t.Fatalf("unexpected typed single-parameter lambda: %#v", fourthLambda.Parameters)
	}
}

func TestParseBlockLambda(t *testing.T) {
	src := `
def vars() Int {
	add = (x Int) -> {
		y Int = x + 1
		return y
	}
	return add(1)
}
`

	program, err := Parse(src)
	if err != nil {
		t.Fatalf("Parse returned error: %v", err)
	}

	fn := program.Functions[0]
	stmt := fn.Body.Statements[0].(*ValStmt)
	lambda, ok := stmt.Values[0].(*LambdaExpr)
	if !ok {
		t.Fatalf("expected lambda expression, got %T", stmt.Values[0])
	}
	if lambda.BlockBody == nil || lambda.Body != nil {
		t.Fatalf("expected block-bodied lambda, got %#v", lambda)
	}
	if len(lambda.BlockBody.Statements) != 2 {
		t.Fatalf("expected 2 statements in lambda block, got %d", len(lambda.BlockBody.Statements))
	}
}

func TestParseLambdaIgnoreParameter(t *testing.T) {
	src := `
def run() Unit {
	pairs.map((key, _) -> key)
	_ -> 1
}
`

	program, err := Parse(src)
	if err != nil {
		t.Fatalf("Parse returned error: %v", err)
	}

	fn := program.Functions[0]
	first := fn.Body.Statements[0].(*ExprStmt)
	firstCall, ok := first.Expr.(*CallExpr)
	if !ok {
		t.Fatalf("expected first statement call expression, got %T", first.Expr)
	}
	firstLambda, ok := firstCall.Args[0].Value.(*LambdaExpr)
	if !ok {
		t.Fatalf("expected first call arg lambda, got %T", firstCall.Args[0].Value)
	}
	if len(firstLambda.Parameters) != 2 || firstLambda.Parameters[0].Name != "key" || firstLambda.Parameters[1].Name != "_" {
		t.Fatalf("unexpected lambda params %#v", firstLambda.Parameters)
	}

	second := fn.Body.Statements[1].(*ExprStmt)
	secondLambda, ok := second.Expr.(*LambdaExpr)
	if !ok {
		t.Fatalf("expected second statement lambda, got %T", second.Expr)
	}
	if len(secondLambda.Parameters) != 1 || secondLambda.Parameters[0].Name != "_" {
		t.Fatalf("unexpected underscore lambda %#v", secondLambda.Parameters)
	}
}

func TestParseTrailingBlockLambda(t *testing.T) {
	src := `
def run() Unit {
	items.map { x -> x + 1 }
}
`

	program, err := Parse(src)
	if err != nil {
		t.Fatalf("Parse returned error: %v", err)
	}

	fn := program.Functions[0]
	stmt := fn.Body.Statements[0].(*ExprStmt)
	call, ok := stmt.Expr.(*CallExpr)
	if !ok {
		t.Fatalf("expected call expression, got %T", stmt.Expr)
	}
	if len(call.Args) != 1 {
		t.Fatalf("expected 1 call arg, got %d", len(call.Args))
	}
	lambda, ok := call.Args[0].Value.(*LambdaExpr)
	if !ok {
		t.Fatalf("expected trailing lambda arg, got %T", call.Args[0].Value)
	}
	if len(lambda.Parameters) != 1 || lambda.Parameters[0].Name != "x" {
		t.Fatalf("unexpected trailing lambda params %#v", lambda.Parameters)
	}
	if lambda.BlockBody == nil || len(lambda.BlockBody.Statements) != 1 {
		t.Fatalf("expected block-bodied trailing lambda, got %#v", lambda)
	}
}

func TestParseContextualMatchLambda(t *testing.T) {
	src := `
def run() Unit {
	options.map(match {
	case SomeX(x) => x + 1
	case NoneX => 0
	})
}
`

	program, err := Parse(src)
	if err != nil {
		t.Fatalf("Parse returned error: %v", err)
	}

	fn := program.Functions[0]
	stmt := fn.Body.Statements[0].(*ExprStmt)
	call, ok := stmt.Expr.(*CallExpr)
	if !ok {
		t.Fatalf("expected call expression, got %T", stmt.Expr)
	}
	matchExpr, ok := call.Args[0].Value.(*MatchExpr)
	if !ok {
		t.Fatalf("expected match expr arg, got %T", call.Args[0].Value)
	}
	if _, ok := matchExpr.Value.(*PlaceholderExpr); !ok {
		t.Fatalf("expected contextual match to use placeholder value, got %T", matchExpr.Value)
	}
}

func TestParseGenericTypeRefs(t *testing.T) {
	src := `
interface Pairer[K, V] {
	def pair(left K, right V) Map[K, V]
}

class Store[T] {
	values List[T]
}

impl Store[T] {
	def init(values List[T]) {
	}
}

def wrap(input Map[Str, List[Int]]) List[Map[Str, Int]] {
	cache Map[Str, List[Int]] = input
	return [cache]
}
`

	program, err := Parse(src)
	if err != nil {
		t.Fatalf("Parse returned error: %v", err)
	}

	assertTypeRef(t, program.Interfaces[0].Methods[0].ReturnType, "Map", "K", "V")
	assertTypeRef(t, program.Classes[0].Fields[0].Type, "List", "T")
	assertTypeRef(t, program.Functions[0].Parameters[0].Type, "Map", "Str", "List")
	if len(program.Functions[0].Parameters[0].Type.Arguments[1].Arguments) != 1 || program.Functions[0].Parameters[0].Type.Arguments[1].Arguments[0].Name != "Int" {
		t.Fatalf("expected nested generic type argument, got %#v", program.Functions[0].Parameters[0].Type)
	}
	assertTypeRef(t, program.Functions[0].ReturnType, "List", "Map")
	if len(program.Functions[0].ReturnType.Arguments[0].Arguments) != 2 {
		t.Fatalf("expected nested return type arguments, got %#v", program.Functions[0].ReturnType)
	}
}

func TestParseFunctionTypeRefs(t *testing.T) {
	src := `
def apply(value Int, f Int -> Int) Int {
	return f(value)
}

def pair(f (Int, Str) -> Bool) Bool {
	return false
}
`

	program, err := Parse(src)
	if err != nil {
		t.Fatalf("Parse returned error: %v", err)
	}

	fnType := program.Functions[0].Parameters[1].Type
	if fnType.ReturnType == nil || len(fnType.ParameterTypes) != 1 {
		t.Fatalf("expected single-parameter function type, got %#v", fnType)
	}
	assertTypeRef(t, fnType.ParameterTypes[0], "Int")
	assertTypeRef(t, fnType.ReturnType, "Int")

	pairType := program.Functions[1].Parameters[0].Type
	if pairType.ReturnType == nil || len(pairType.ParameterTypes) != 2 {
		t.Fatalf("expected two-parameter function type, got %#v", pairType)
	}
	assertTypeRef(t, pairType.ParameterTypes[0], "Int")
	assertTypeRef(t, pairType.ParameterTypes[1], "Str")
	assertTypeRef(t, pairType.ReturnType, "Bool")
}

func TestParseElseIf(t *testing.T) {
	src := `
def classify(a Int) Bool {
	if a == 1 {
		return a == 1
	} else if a == 2 {
		return a == 2
	} else {
		return a == 3
	}
}
`

	program, err := Parse(src)
	if err != nil {
		t.Fatalf("Parse returned error: %v", err)
	}

	fn := program.Functions[0]
	ifStmt, ok := fn.Body.Statements[0].(*IfStmt)
	if !ok {
		t.Fatalf("expected first statement to be if, got %T", fn.Body.Statements[0])
	}
	if ifStmt.ElseIf == nil {
		t.Fatalf("expected else-if branch to be present")
	}
	if ifStmt.ElseIf.Else == nil {
		t.Fatalf("expected final else block on else-if chain")
	}
}

func TestParseIfOptionBinding(t *testing.T) {
	src := `
def run(value Option[Int]) Unit {
	if let Some(item) = value {
		OS.println(item)
	}
}
`

	program, err := Parse(src)
	if err != nil {
		t.Fatalf("Parse returned error: %v", err)
	}

	fn := program.Functions[0]
	ifStmt, ok := fn.Body.Statements[0].(*IfStmt)
	if !ok {
		t.Fatalf("expected first statement to be if, got %T", fn.Body.Statements[0])
	}
	constructor, ok := ifStmt.Pattern.(*ConstructorPattern)
	if !ok {
		t.Fatalf("expected constructor pattern, got %T", ifStmt.Pattern)
	}
	if len(constructor.Path) != 1 || constructor.Path[0] != "Some" {
		t.Fatalf("expected Some(...) pattern, got %#v", constructor.Path)
	}
	binding, ok := constructor.Args[0].(*BindingPattern)
	if !ok || binding.Name != "item" {
		t.Fatalf("expected binding name 'item', got %#v", constructor.Args)
	}
	if ifStmt.PatternValue == nil {
		t.Fatalf("expected pattern value to be populated")
	}
	if ifStmt.Condition != nil {
		t.Fatalf("expected boolean condition to be empty for pattern if")
	}
}

func TestParseIfOptionDestructuring(t *testing.T) {
	src := `
def run(value Option[(Int, Str, Bool)]) Unit {
	if let Some(item) = value {
		let (_, name, _) = item
		OS.println(name)
	}
}
`

	program, err := Parse(src)
	if err != nil {
		t.Fatalf("Parse returned error: %v", err)
	}
	ifStmt := program.Functions[0].Body.Statements[0].(*IfStmt)
	constructor, ok := ifStmt.Pattern.(*ConstructorPattern)
	if !ok {
		t.Fatalf("expected constructor pattern, got %T", ifStmt.Pattern)
	}
	binding, ok := constructor.Args[0].(*BindingPattern)
	if !ok || binding.Name != "item" {
		t.Fatalf("expected binding name 'item', got %#v", constructor.Args)
	}
	if len(ifStmt.Then.Statements) == 0 {
		t.Fatalf("expected if body statements")
	}
	if _, ok := ifStmt.Then.Statements[0].(*ValStmt); !ok {
		t.Fatalf("expected first if-body statement to destructure item, got %T", ifStmt.Then.Statements[0])
	}
}

func TestParseIfLetClauseBlock(t *testing.T) {
	src := `
def run(left Option[Int], right Option[Int]) Unit {
	if let {
		Some(a) = left
		Some(b) = right
	} {
		OS.println(a + b)
	}
}
`

	program, err := Parse(src)
	if err != nil {
		t.Fatalf("Parse returned error: %v", err)
	}
	ifStmt := program.Functions[0].Body.Statements[0].(*IfStmt)
	if len(ifStmt.PatternClauses) != 2 {
		t.Fatalf("expected 2 pattern clauses, got %d", len(ifStmt.PatternClauses))
	}
	if ifStmt.Pattern != nil || ifStmt.PatternValue != nil {
		t.Fatalf("expected single-pattern fields to be empty for grouped if let")
	}
}

func TestParseIfLetConditionChain(t *testing.T) {
	src := `
def run(left Option[Int], right Result[Int, Str]) Unit {
	if let Some(a) = left && let Ok(b) = right && b == 5 {
		OS.println(a + b)
	}
}
`

	program, err := Parse(src)
	if err != nil {
		t.Fatalf("Parse returned error: %v", err)
	}
	ifStmt := program.Functions[0].Body.Statements[0].(*IfStmt)
	if len(ifStmt.ConditionClauses) != 3 {
		t.Fatalf("expected 3 condition clauses, got %d", len(ifStmt.ConditionClauses))
	}
	if ifStmt.Pattern != nil || ifStmt.PatternValue != nil || len(ifStmt.PatternClauses) > 0 {
		t.Fatalf("expected legacy if-let fields to be empty for chained if let")
	}
	if ifStmt.ConditionClauses[0].Pattern == nil || ifStmt.ConditionClauses[0].Value == nil {
		t.Fatalf("expected first clause to be refutable")
	}
	if ifStmt.ConditionClauses[2].Condition == nil {
		t.Fatalf("expected final clause to be boolean")
	}
}

func TestParseIfPatternRequiresLet(t *testing.T) {
	src := `
def run(value Option[Int]) Unit {
	if Some(item) = value {
		OS.println(item)
	}
}
`

	_, err := Parse(src)
	if err == nil {
		t.Fatalf("expected parse error, got nil")
	}
	if !strings.Contains(err.Error(), "require 'let'") {
		t.Fatalf("expected let-specific parse error, got %v", err)
	}
}

func TestParseLetClauseBlock(t *testing.T) {
	src := `
def run(left Option[Int], right Option[Int]) Int {
	let {
		Some(a) = left
		Some(b) = right
	} else {
		return 0
	}
	return a + b
}
`

	program, err := Parse(src)
	if err != nil {
		t.Fatalf("Parse returned error: %v", err)
	}
	stmt, ok := program.Functions[0].Body.Statements[0].(*LetElseStmt)
	if !ok {
		t.Fatalf("expected first statement to be let-else, got %T", program.Functions[0].Body.Statements[0])
	}
	if len(stmt.Clauses) != 2 {
		t.Fatalf("expected 2 let clauses, got %d", len(stmt.Clauses))
	}
	if stmt.Pattern != nil || stmt.Value != nil {
		t.Fatalf("expected single-pattern fields to be empty for grouped let")
	}
}

func TestParseBareUnwrapStmtRemoved(t *testing.T) {
	src := `
def run(value Result[Int, Str]) Result[Int, Str] {
	unwrap item <- value
	return Ok(item + 1)
}
`

	if _, err := Parse(src); err == nil {
		t.Fatalf("expected parse error for removed bare unwrap syntax")
	} else if !strings.Contains(err.Error(), "removed") {
		t.Fatalf("expected removed-syntax error, got %v", err)
	}
}

func TestParseGuardStmt(t *testing.T) {
	src := `
def run(value Option[Int]) Result[Int, Str] {
	unwrap item <- value else Err("missing")
	return Ok(item + 1)
}
`

	program, err := Parse(src)
	if err != nil {
		t.Fatalf("Parse returned error: %v", err)
	}

	fn := program.Functions[0]
	stmt, ok := fn.Body.Statements[0].(*GuardStmt)
	if !ok {
		t.Fatalf("expected first statement to be unwrap-with-else, got %T", fn.Body.Statements[0])
	}
	if stmt.Fallback == nil || len(stmt.Fallback.Statements) != 1 {
		t.Fatalf("expected fallback block on unwrap statement")
	}
}

func TestParseUnwrapBlockElseStmt(t *testing.T) {
	src := `
def run(b Option[Int], d Option[Int]) Result[Int, Str] {
	unwrap {
		a <- b
		c <- d
	} else {
		Err("missing")
	}
	return Ok(a + c)
}
`

	program, err := Parse(src)
	if err != nil {
		t.Fatalf("Parse returned error: %v", err)
	}

	fn := program.Functions[0]
	stmt, ok := fn.Body.Statements[0].(*GuardBlockStmt)
	if !ok {
		t.Fatalf("expected first statement to be unwrap-else block, got %T", fn.Body.Statements[0])
	}
	if len(stmt.Clauses) != 2 {
		t.Fatalf("expected 2 unwrap clauses, got %d", len(stmt.Clauses))
	}
}

func TestParseBareUnwrapBlockRemoved(t *testing.T) {
	src := `
def run(b Option[Int], d Option[Int]) Option[Int] {
	unwrap {
		a <- b
		c <- d
	}
	return Some(a + c)
}
`

	if _, err := Parse(src); err == nil {
		t.Fatalf("expected parse error for removed bare unwrap block syntax")
	} else if !strings.Contains(err.Error(), "removed") {
		t.Fatalf("expected removed-syntax error, got %v", err)
	}
}

func TestParseBareUnwrapStmtRejected(t *testing.T) {
	src := `
def run(value Option[Int]) Option[Int] {
	item <- value
	return Some(item)
}
`

	if _, err := Parse(src); err == nil {
		t.Fatalf("expected parse error for bare unwrap binding")
	}
}

func TestParseTopLevelEnumImplsRejectCaseMethods(t *testing.T) {
	src := `
enum Either[L, R] {
	case Left {
		value L
	}
	case Right {
		value R
	}
}

impl Either[L, R] {
	def isLeft() Bool = false
}

impl Either[L, R].Left {
	def isLeft() Bool = true
}
`
	if _, err := Parse(src); err == nil || err.Error() != "enum cases cannot declare methods; move methods to enum 'Either'" {
		t.Fatalf("expected enum case impl parse error, got %v", err)
	}
}

func TestAttachSourceSpans(t *testing.T) {
	src := `
def sample(a Int) Bool {
	value = function(a)
	return value == a
}
`

	program, err := Parse(src)
	if err != nil {
		t.Fatalf("Parse returned error: %v", err)
	}

	if program.Span.Start.Line == 0 || program.Span.End.Line == 0 {
		t.Fatalf("expected program span to be populated, got %#v", program.Span)
	}

	fn := program.Functions[0]
	if fn.Span.Start.Line != 2 {
		t.Fatalf("expected function to start on line 2, got %#v", fn.Span)
	}

	stmt := fn.Body.Statements[0].(*ValStmt)
	if stmt.Span.Start.Line != 3 {
		t.Fatalf("expected immutable binding statement to start on line 3, got %#v", stmt.Span)
	}

	call := stmt.Values[0].(*CallExpr)
	if call.Span.Start.Line != 3 || call.Span.End.Line != 3 {
		t.Fatalf("expected call span on line 3, got %#v", call.Span)
	}

	retStmt := fn.Body.Statements[1].(*ReturnStmt)
	if retStmt.Span.Start.Line != 4 {
		t.Fatalf("expected return statement span on line 4, got %#v", retStmt.Span)
	}
}
