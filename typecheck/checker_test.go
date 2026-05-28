package typecheck

import (
	"os"
	"path/filepath"
	"strings"
	"testing"

	"a-lang/module"
	"a-lang/parser"
)

func parseProgram(t *testing.T, src string) *parser.Program {
	t.Helper()
	program, err := parser.Parse(src)
	if err != nil {
		t.Fatalf("Parse returned error: %v", err)
	}
	return program
}

func writeModuleFile(t *testing.T, path, src string) {
	t.Helper()
	if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
		t.Fatalf("MkdirAll failed: %v", err)
	}
	if err := os.WriteFile(path, []byte(src), 0o644); err != nil {
		t.Fatalf("WriteFile failed: %v", err)
	}
}

func TestAnalyzeValidProgram(t *testing.T) {
	src := `
def add(a Int, b Int) Int {
	return a + b
}

def run(input Int) Bool {
	total Int = add(input, 1)
	var copy Int = total
	copy += 1

	if copy > input {
		return true
	}

	return false
}
`

	result := Analyze(parseProgram(t, src))
	if len(result.Diagnostics) != 0 {
		t.Fatalf("expected no diagnostics, got %#v", result.Diagnostics)
	}
}

func TestAnalyzeTypeMismatch(t *testing.T) {
	src := `
def run() Bool {
	count Int = "hello"
	return true
}
`

	result := Analyze(parseProgram(t, src))
	if len(result.Diagnostics) == 0 || result.Diagnostics[0].Code != "type_mismatch" {
		t.Fatalf("expected type_mismatch diagnostic, got %#v", result.Diagnostics)
	}
}

func TestAnalyzeInvalidConditionAndReturn(t *testing.T) {
	src := `
def run() Bool {
	if 1 {
		return 7
	}

	return false
}
`

	result := Analyze(parseProgram(t, src))
	if len(result.Diagnostics) != 2 {
		t.Fatalf("expected 2 diagnostics, got %#v", result.Diagnostics)
	}
	if result.Diagnostics[0].Code != "invalid_condition_type" {
		t.Fatalf("unexpected first diagnostic %#v", result.Diagnostics[0])
	}
	if result.Diagnostics[1].Code != "invalid_return_type" {
		t.Fatalf("unexpected second diagnostic %#v", result.Diagnostics[1])
	}
}

func TestAnalyzeInvalidCallAndAssignment(t *testing.T) {
	src := `
def add(a Int, b Int) Int {
	return a + b
}

def run() Int {
	value Int = add(true)
	fixed Int = 1
	fixed := 2
	return value
}
`

	result := Analyze(parseProgram(t, src))
	if len(result.Diagnostics) != 3 {
		t.Fatalf("expected 3 diagnostics, got %#v", result.Diagnostics)
	}
	if result.Diagnostics[0].Code != "invalid_argument_count" {
		t.Fatalf("unexpected first diagnostic %#v", result.Diagnostics[0])
	}
	if result.Diagnostics[1].Code != "invalid_argument_type" {
		t.Fatalf("unexpected second diagnostic %#v", result.Diagnostics[1])
	}
	if result.Diagnostics[2].Code != "assign_immutable" {
		t.Fatalf("unexpected third diagnostic %#v", result.Diagnostics[2])
	}
}

func TestAnalyzeMatchStmt(t *testing.T) {
	src := `
enum OptionX[T] {
	case NoneX
	case SomeX {
		value T
	}
}

def run(value OptionX[Int]) Int {
	match value {
		case SomeX(x) => {
			return x
		}
		case OptionX.NoneX => {
			return 0
		}
	}
	return 0
}
`

	result := Analyze(parseProgram(t, src))
	if len(result.Diagnostics) != 0 {
		t.Fatalf("expected no diagnostics, got %#v", result.Diagnostics)
	}
}

func TestAnalyzeMatchGuard(t *testing.T) {
	src := `
enum OptionX[T] {
	case NoneX
	case SomeX {
		value T
	}
}

def run(value OptionX[Int]) Int =
	match value {
		case SomeX(x) if x > 10 => x
		case SomeX(_) => 10
		case OptionX.NoneX => 0
	}
`

	result := Analyze(parseProgram(t, src))
	if len(result.Diagnostics) != 0 {
		t.Fatalf("expected no diagnostics, got %#v", result.Diagnostics)
	}
}

func TestAnalyzeMatchGuardRequiresBool(t *testing.T) {
	src := `
enum OptionX[T] {
	case NoneX
	case SomeX {
		value T
	}
}

def run(value OptionX[Int]) Int =
	match value {
		case SomeX(x) if x => x
		case OptionX.NoneX => 0
	}
`

	result := Analyze(parseProgram(t, src))
	if len(result.Diagnostics) == 0 {
		t.Fatalf("expected diagnostics, got none")
	}
	found := false
	for _, diag := range result.Diagnostics {
		if diag.Code == "invalid_condition_type" {
			found = true
			break
		}
	}
	if !found {
		t.Fatalf("expected invalid_condition_type diagnostic, got %#v", result.Diagnostics)
	}
}

func TestAnalyzeMatchStmtRequiresEnumExhaustiveness(t *testing.T) {
	src := `
enum OptionX[T] {
	case NoneX
	case SomeX {
		value T
	}
}

def run(value OptionX[Int]) Int {
	match value {
		case SomeX(x) => {
			return x
		}
	}
	return 0
}
`

	result := Analyze(parseProgram(t, src))
	if len(result.Diagnostics) == 0 {
		t.Fatalf("expected diagnostics, got none")
	}
	if result.Diagnostics[0].Code != "non_exhaustive_match" {
		t.Fatalf("unexpected diagnostic %#v", result.Diagnostics[0])
	}
}

func TestAnalyzeMatchExprRequiresEnumExhaustiveness(t *testing.T) {
	src := `
enum OptionX[T] {
	case NoneX
	case SomeX {
		value T
	}
}

def run(value OptionX[Int]) Int =
	match value {
		case SomeX(x) => x
	}
`

	result := Analyze(parseProgram(t, src))
	if len(result.Diagnostics) == 0 {
		t.Fatalf("expected diagnostics, got none")
	}
	if result.Diagnostics[0].Code != "non_exhaustive_match" {
		t.Fatalf("unexpected diagnostic %#v", result.Diagnostics[0])
	}
}

func TestAnalyzeMatchClassExtractor(t *testing.T) {
	src := `
class PairBox {
	left Int
	right Int
}

def run(value PairBox) Int {
	match value {
		case PairBox(left, right) => {
			return left + right
		}
	}
	return 0
}
`

	result := Analyze(parseProgram(t, src))
	if len(result.Diagnostics) != 0 {
		t.Fatalf("expected no diagnostics, got %#v", result.Diagnostics)
	}
}

func TestAnalyzeMatchGenericEnumAndClassExtractor(t *testing.T) {
	src := `
enum OptionX[T] {
	case NoneX
	case SomeX {
		value T
	}
}

class Box[T] {
	value T
}

def unwrapSome(value OptionX[Int]) Int =
	match value {
		case SomeX(x) => x + 1
		case OptionX.NoneX => 0
	}

def unwrapBox(value Box[Int]) Int =
	match value {
		case Box(x) => x + 2
	}
`

	result := Analyze(parseProgram(t, src))
	if len(result.Diagnostics) != 0 {
		t.Fatalf("expected no diagnostics, got %#v", result.Diagnostics)
	}
}

func TestAnalyzeMatchRecordExtractor(t *testing.T) {
	src := `
record Amount {
	count Int
	label Str
}

def run(value Amount) Int {
	match value {
		case Amount(count, label) => {
			return count
		}
	}
	return 0
}
`

	result := Analyze(parseProgram(t, src))
	if len(result.Diagnostics) != 0 {
		t.Fatalf("expected no diagnostics, got %#v", result.Diagnostics)
	}
}

func TestAnalyzeMatchTuplePatternDoesNotDestructureClass(t *testing.T) {
	src := `
class PairBox {
	left Int
	right Int
}

def run(value PairBox) Int =
	match value {
		case (left, right) => left + right
	}
`

	result := Analyze(parseProgram(t, src))
	if len(result.Diagnostics) == 0 {
		t.Fatalf("expected diagnostics, got none")
	}
	found := false
	for _, diag := range result.Diagnostics {
		if diag.Code == "invalid_match_pattern" {
			found = true
			break
		}
	}
	if !found {
		t.Fatalf("expected invalid_match_pattern, got %#v", result.Diagnostics)
	}
}

func TestAnalyzeNestedMatchExhaustiveness(t *testing.T) {
	src := `
enum BoolBox {
	case Wrap {
		value Bool
	}
	case Empty
}

def run(value BoolBox) Int =
	match value {
		case Wrap(true) => 1
		case Wrap(false) => 0
		case BoolBox.Empty => 2
	}
`

	result := Analyze(parseProgram(t, src))
	if len(result.Diagnostics) != 0 {
		t.Fatalf("expected no diagnostics, got %#v", result.Diagnostics)
	}
}

func TestAnalyzeUnreachableNestedMatchCase(t *testing.T) {
	src := `
enum BoolBox {
	case Wrap {
		value Bool
	}
	case Empty
}

def run(value BoolBox) Int =
	match value {
		case Wrap(_) => 1
		case Wrap(true) => 2
		case BoolBox.Empty => 0
	}
`

	result := Analyze(parseProgram(t, src))
	found := false
	for _, diag := range result.Diagnostics {
		if diag.Code == "unreachable_match_case" {
			found = true
			break
		}
	}
	if !found {
		t.Fatalf("expected unreachable_match_case, got %#v", result.Diagnostics)
	}
}

func TestAnalyzeNestedMatchExhaustivenessFallsBackPastDomainCap(t *testing.T) {
	src := `
enum BigBox {
	case Wrap {
		value (Bool, Bool, Bool, Bool, Bool, Bool)
	}
}

def run(value BigBox) Int =
	match value {
		case Wrap((true, true, true, true, true, true)) => 1
	}
`

	result := Analyze(parseProgram(t, src))
	if len(result.Diagnostics) != 0 {
		t.Fatalf("expected no diagnostics, got %#v", result.Diagnostics)
	}
}

func TestAnalyzeMatchTypePattern(t *testing.T) {
	src := `
interface WorkerLike {
}

class Worker with WorkerLike {
}

class Other with WorkerLike {
}

def run(value WorkerLike) Int {
	match value {
		case worker Worker => {
			return 1
		}
		case _ Other => {
			return 2
		}
		case _ => {
			return 3
		}
	}
	return 0
}
`

	result := Analyze(parseProgram(t, src))
	if len(result.Diagnostics) != 0 {
		t.Fatalf("expected no diagnostics, got %#v", result.Diagnostics)
	}
}

func TestAnalyzeMatchErasedGenericTypePattern(t *testing.T) {
	src := `
enum OptionX[T] {
	case NoneX
	case SomeX {
		value T
	}
}

class Box[T] {
	value T
}

def describe(value OptionX[Int]) Int =
	match value {
		case some OptionX => 1
	}

def describeBox(value Box[Int]) Int =
	match value {
		case box Box => 2
	}
`

	result := Analyze(parseProgram(t, src))
	if len(result.Diagnostics) != 0 {
		t.Fatalf("expected no diagnostics, got %#v", result.Diagnostics)
	}
}

func TestAnalyzeMatchTypePatternRejectsGenericArgs(t *testing.T) {
	src := `
class Box[T] {
	value T
}

def describe(value Box[Int]) Int =
	match value {
		case _ Box[Int] => 1
		case _ => 0
	}
`

	result := Analyze(parseProgram(t, src))
	if len(result.Diagnostics) == 0 {
		t.Fatalf("expected diagnostics, got none")
	}
	found := false
	for _, diag := range result.Diagnostics {
		if diag.Code == "invalid_match_pattern" {
			found = true
			break
		}
	}
	if !found {
		t.Fatalf("expected invalid_match_pattern diagnostic, got %#v", result.Diagnostics)
	}
}

func TestAnalyzeConstructorFieldAssignmentAllowsEquals(t *testing.T) {
	src := `
class Box {
	value Int
}

impl Box {
	def init(value Int) {
		this.value = value
	}
}

def run() Int {
	box Box = Box(2)
	return box.value
}
`

	result := Analyze(parseProgram(t, src))
	if len(result.Diagnostics) != 0 {
		t.Fatalf("expected no diagnostics, got %#v", result.Diagnostics)
	}
}

func TestAnalyzeTupleDestructuring(t *testing.T) {
	src := `
def run() Int {
	a (Int, Int) = (1, 2)
	b (Int, Int) = a
	c = a
	let (left Int, right Int) = c
	let (otherLeft Int, otherRight Int) = b
	return left + right + otherLeft + otherRight
}
`

	result := Analyze(parseProgram(t, src))
	if len(result.Diagnostics) != 0 {
		t.Fatalf("expected no diagnostics, got %#v", result.Diagnostics)
	}
}

func TestAnalyzeTupleMemberAccess(t *testing.T) {
	src := `
def run() Int {
	pair = (1, "x")
	return pair._1
}
`

	result := Analyze(parseProgram(t, src))
	if len(result.Diagnostics) != 1 || result.Diagnostics[0].Code != "unknown_member" {
		t.Fatalf("expected unknown_member diagnostic, got %#v", result.Diagnostics)
	}
}

func TestAnalyzeArrayIndexing(t *testing.T) {
	src := `
def run(values Array[Int]) Int {
	first Int = values[0]
	values[1] := first + 2
	return values[1]
}
`

	result := Analyze(parseProgram(t, src))
	if len(result.Diagnostics) != 0 {
		t.Fatalf("expected no diagnostics, got %#v", result.Diagnostics)
	}
}

func TestAnalyzeArrayElementConstruction(t *testing.T) {
	src := `
class Box {
	value Int
}

def run() Int {
	values Array[Int] = Array(4, 5, 6)
	boxes Array[Box] = Array(Box(7), Box(8))
	return values[0] + values[2] + boxes[1].value
}
`

	result := Analyze(parseProgram(t, src))
	if len(result.Diagnostics) != 0 {
		t.Fatalf("expected no diagnostics, got %#v", result.Diagnostics)
	}
}

func TestAnalyzeInvalidIndexing(t *testing.T) {
	src := `
def fromSet(values Set[Int]) Int {
	return values[0]
}

def fromArray(values Array[Int]) Int {
	return values[true]
}
`

	result := Analyze(parseProgram(t, src))
	if len(result.Diagnostics) != 2 {
		t.Fatalf("expected 2 diagnostics, got %#v", result.Diagnostics)
	}
	if result.Diagnostics[0].Code != "invalid_index_target" {
		t.Fatalf("unexpected first diagnostic %#v", result.Diagnostics[0])
	}
	if result.Diagnostics[1].Code != "invalid_index_type" {
		t.Fatalf("unexpected second diagnostic %#v", result.Diagnostics[1])
	}
}

func TestAnalyzeClassMembersAndConstructors(t *testing.T) {
	src := `
class Counter {
	hidden var count Int
}

impl Counter {
	def init(count Int) {
		this.count = count
	}

	def inc() Int {
		this.count += 1
		return this.count
	}
}

def run() Int {
	counter Counter = Counter(1)
	return counter.inc()
}
`

	result := Analyze(parseProgram(t, src))
	if len(result.Diagnostics) != 0 {
		t.Fatalf("expected no diagnostics, got %#v", result.Diagnostics)
	}
}

func TestAnalyzeImplicitPrimaryConstructorAndThisDelegation(t *testing.T) {
	src := `
class Counter {
	count Int
	label Str
	hidden var seen Bool = false
}

impl Counter {
	def init(seed Int) {
		init(count = seed, label = "ok")
	}
}

def run() Int {
	left Counter = Counter(1, "x")
	right Counter = Counter(seed = 3)
	return left.count + right.count
}
`

	result := Analyze(parseProgram(t, src))
	if len(result.Diagnostics) != 0 {
		t.Fatalf("expected no diagnostics, got %#v", result.Diagnostics)
	}
}

func TestAnalyzeRecordRejectsMutableFields(t *testing.T) {
	src := `
record Amount {
	var amount Int = 1
}
`

	result := Analyze(parseProgram(t, src))
	if len(result.Diagnostics) == 0 {
		t.Fatalf("expected diagnostics for mutable record field")
	}
	if result.Diagnostics[0].Code != "invalid_record_field" {
		t.Fatalf("unexpected diagnostic %#v", result.Diagnostics[0])
	}
}

func TestAnalyzeRecordUpdateExpr(t *testing.T) {
	src := `
record Amount {
	amount Int
	description Str
}

def run() Int {
	value = Amount(10, "x")
	updated = value with { amount = 42 }
	return updated.amount
}
`

	result := Analyze(parseProgram(t, src))
	if len(result.Diagnostics) != 0 {
		t.Fatalf("expected no diagnostics, got %#v", result.Diagnostics)
	}
}

func TestAnalyzeAnonymousInterfaceExpr(t *testing.T) {
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

	result := Analyze(parseProgram(t, src))
	if len(result.Diagnostics) != 0 {
		t.Fatalf("expected no diagnostics, got %#v", result.Diagnostics)
	}
}

func TestAnalyzeAnonymousRecordExpr(t *testing.T) {
	src := `
def describe(user { name Str, age Int }) Int {
	return user.age
}

def makeCounter(base Int) { count Int, next Int } = {
	return record(base, base + 1)
}

def run() Int {
	full = record {
		name = "Ana"
		age = 10
		city = "NYC"
	}
	narrow { name Str, age Int } = full
	positional { name Str, age Int } = record("Ben", 12)
	counter { count Int, next Int } = makeCounter(5)
	describe(full)
	describe(record("Cara", 14))
	return full.age + narrow.age + positional.age + counter.next
}
`

	result := Analyze(parseProgram(t, src))
	if len(result.Diagnostics) != 0 {
		t.Fatalf("expected no diagnostics, got %#v", result.Diagnostics)
	}
}

func TestAnalyzeAnonymousRecordToClassAndRecord(t *testing.T) {
	src := `
record User {
	name Str
	age Int
}

class Person {
	name Str
	age Int
	city Str = "NYC"
}

class Team {
	name Str
	owner Person
}

impl Team {
	def init(name Str, owner Person) {
		this.name = name
		this.owner = owner
	}
}

def run() Int {
	userRecord = record {
		name = "Ana"
		age = 10
	}
	user User = User(userRecord)
	person Person = Person(record("Ben", 12))
	team Team = Team(record {
		name = "Core"
		owner = Person(record {
			name = "Cy"
			age = 7
		})
	})
	return user.age + person.age + team.owner.age
}
`

	result := Analyze(parseProgram(t, src))
	if len(result.Diagnostics) != 0 {
		t.Fatalf("expected no diagnostics, got %#v", result.Diagnostics)
	}
}

func TestAnalyzeRecordAndClassDestructuring(t *testing.T) {
	src := `
record Pair {
	left Int
	right Str
}

class Box {
	value Int
	label Str
}

def run() Int {
	let (a Int, b Str) = Pair(5, "x")
	let (c Int, d Str) = Box(7, "y")
	return a + c
}
`

	result := Analyze(parseProgram(t, src))
	if len(result.Diagnostics) != 0 {
		t.Fatalf("expected no diagnostics, got %#v", result.Diagnostics)
	}
}

func TestAnalyzeDestructuringSkipBinding(t *testing.T) {
	src := `
record Triple {
	first Int
	middle Str
	last Str
}

def run() Int {
	let (a Int, _, c Str) = Triple(1, "drop", "keep")
	return a
}
`

	result := Analyze(parseProgram(t, src))
	if len(result.Diagnostics) != 0 {
		t.Fatalf("expected no diagnostics, got %#v", result.Diagnostics)
	}
}

func TestAnalyzeClassDestructuringRejectsPrivateFields(t *testing.T) {
	src := `
class Box {
	value Int
	hidden extra Str = "x"
}

def run() Int {
	let (a Int, b Str) = Box(7)
	return a
}
`

	result := Analyze(parseProgram(t, src))
	if len(result.Diagnostics) == 0 {
		t.Fatalf("expected diagnostics, got none")
	}
	found := false
	for _, diag := range result.Diagnostics {
		if diag.Code == "invalid_binding_count" {
			found = true
			break
		}
	}
	if !found {
		t.Fatalf("expected invalid_binding_count diagnostic, got %#v", result.Diagnostics)
	}
}

func TestAnalyzeObjectSingletonAccess(t *testing.T) {
	src := `
object A {
	def value() Int = 5
	def test(a Int) Int = a + this.value()
}

def run() Int {
	return A.test(2)
}
`

	result := Analyze(parseProgram(t, src))
	if len(result.Diagnostics) != 0 {
		t.Fatalf("expected no diagnostics, got %#v", result.Diagnostics)
	}
}

func TestAnalyzeObjectApplyCall(t *testing.T) {
	src := `
object Range {
	def apply(end Int) Int = end
}

def run() Int {
	return Range.apply(5)
}
`

	result := Analyze(parseProgram(t, src))
	if len(result.Diagnostics) != 0 {
		t.Fatalf("expected no diagnostics, got %#v", result.Diagnostics)
	}
}

func TestAnalyzeObjectDirectCall(t *testing.T) {
	src := `
object Range {
	def apply(end Int) Int = end
}

def run() Int {
	return Range(5)
}
`

	result := Analyze(parseProgram(t, src))
	found := false
	for _, diag := range result.Diagnostics {
		if diag.Code == "invalid_call_target" && strings.Contains(diag.Message, "object 'Range' is not callable") {
			found = true
			break
		}
	}
	if !found {
		t.Fatalf("expected invalid_call_target diagnostic, got %#v", result.Diagnostics)
	}
}

func TestAnalyzeForTupleRangeSyntaxRejected(t *testing.T) {
	src := `
def run() Unit {
	for item <- ("x", 4) {
		OS.println(item)
	}
}
`

	result := Analyze(parseProgram(t, src))
	found := false
	for _, diag := range result.Diagnostics {
		if diag.Code == "invalid_for_range" {
			found = true
			break
		}
	}
	if !found {
		t.Fatalf("expected invalid_for_range diagnostic, got %#v", result.Diagnostics)
	}
}

func TestAnalyzePrivateFieldInferenceInClassAndObject(t *testing.T) {
	src := `
class Box {
	hidden count = 1
	hidden var total = 0
}

impl Box {
	def bump() Unit {
		this.total += this.count
	}

	def value() Int = this.total
}

object Greeter {
	hidden hello = "Hello"
}

impl Greeter {
	def greet() Str = this.hello
}
`

	result := Analyze(parseProgram(t, src))
	if len(result.Diagnostics) != 0 {
		t.Fatalf("expected no diagnostics, got %#v", result.Diagnostics)
	}
}

func TestAnalyzeClassApplyCall(t *testing.T) {
	src := `
class Adder {
	amount Int
}

impl Adder {
	def apply(value Int) Int = amount + value
}

def run() Int {
	adder Adder = Adder(5)
	return adder.apply(7)
}
`

	result := Analyze(parseProgram(t, src))
	if len(result.Diagnostics) != 0 {
		t.Fatalf("expected no diagnostics, got %#v", result.Diagnostics)
	}
}

func TestAnalyzeRecordApplyCall(t *testing.T) {
	src := `
record Adder {
	amount Int
}

impl Adder {
	def apply(value Int) Int = amount + value
}

def run() Int {
	adder Adder = Adder(5)
	return adder.apply(7)
}
`

	result := Analyze(parseProgram(t, src))
	if len(result.Diagnostics) != 0 {
		t.Fatalf("expected no diagnostics, got %#v", result.Diagnostics)
	}
}

func TestAnalyzeModuleAllowsPrivateClassWithinSamePackage(t *testing.T) {
	dir := t.TempDir()
	writeModuleFile(t, filepath.Join(dir, "app.lum"), `
module app

hidden class Hidden {
}

impl Hidden {
	def value() Int = 7
}
`)
	writeModuleFile(t, filepath.Join(dir, "main.lum"), `
module app

import app

def run() Int {
	value Hidden = app.Hidden()
	return value.value()
}
`)

	mod, err := module.Load(filepath.Join(dir, "main.lum"))
	if err != nil {
		t.Fatalf("Load returned error: %v", err)
	}
	result := AnalyzeModule(mod)
	if len(result.Diagnostics) != 0 {
		t.Fatalf("expected no diagnostics, got %#v", result.Diagnostics)
	}
}

func TestAnalyzeModuleRejectsPrivateClassAcrossPackages(t *testing.T) {
	dir := t.TempDir()
	writeModuleFile(t, filepath.Join(dir, "lib.lum"), `
module lib

hidden class Hidden {
}

impl Hidden {
	def value() Int = 7
}
`)
	writeModuleFile(t, filepath.Join(dir, "main.lum"), `
module app

import lib

def run() Int {
	value = lib.Hidden()
	return 0
}
`)

	mod, err := module.Load(filepath.Join(dir, "main.lum"))
	if err != nil {
		t.Fatalf("Load returned error: %v", err)
	}
	result := AnalyzeModule(mod)
	if len(result.Diagnostics) == 0 {
		t.Fatalf("expected diagnostics, got none")
	}
	found := false
	for _, diag := range result.Diagnostics {
		if diag.Code == "unknown_member" {
			found = true
			break
		}
	}
	if !found {
		t.Fatalf("expected unknown_member diagnostic, got %#v", result.Diagnostics)
	}
}

func TestAnalyzeModuleRejectsPrivateFunctionWithinSamePackage(t *testing.T) {
	dir := t.TempDir()
	writeModuleFile(t, filepath.Join(dir, "app.lum"), `
module app

hidden def localValue() Int = 7
`)
	writeModuleFile(t, filepath.Join(dir, "main.lum"), `
module app

import app

def run() Int {
	return app.localValue()
}
`)

	mod, err := module.Load(filepath.Join(dir, "main.lum"))
	if err != nil {
		t.Fatalf("Load returned error: %v", err)
	}
	result := AnalyzeModule(mod)
	if len(result.Diagnostics) == 0 {
		t.Fatalf("expected diagnostics, got none")
	}
	found := false
	for _, diag := range result.Diagnostics {
		if diag.Code == "unknown_member" {
			found = true
			break
		}
	}
	if !found {
		t.Fatalf("expected unknown_member diagnostic, got %#v", result.Diagnostics)
	}
}

func TestAnalyzeModuleRejectsPrivateFunctionAcrossPackages(t *testing.T) {
	dir := t.TempDir()
	writeModuleFile(t, filepath.Join(dir, "lib.lum"), `
module lib

hidden def localValue() Int = 7
`)
	writeModuleFile(t, filepath.Join(dir, "main.lum"), `
module app

import lib

def run() Int {
	return lib.localValue()
}
`)

	mod, err := module.Load(filepath.Join(dir, "main.lum"))
	if err != nil {
		t.Fatalf("Load returned error: %v", err)
	}
	result := AnalyzeModule(mod)
	if len(result.Diagnostics) == 0 {
		t.Fatalf("expected diagnostics, got none")
	}
	found := false
	for _, diag := range result.Diagnostics {
		if diag.Code == "unknown_member" {
			found = true
			break
		}
	}
	if !found {
		t.Fatalf("expected unknown_member diagnostic, got %#v", result.Diagnostics)
	}
}

func TestAnalyzeModuleAllowsPublicFunctionAcrossPackages(t *testing.T) {
	dir := t.TempDir()
	writeModuleFile(t, filepath.Join(dir, "lib.lum"), `
module lib

public def exportedValue() Int = 7
`)
	writeModuleFile(t, filepath.Join(dir, "main.lum"), `
module app

import lib

def run() Int {
	return lib.exportedValue()
}
`)

	mod, err := module.Load(filepath.Join(dir, "main.lum"))
	if err != nil {
		t.Fatalf("Load returned error: %v", err)
	}
	result := AnalyzeModule(mod)
	if len(result.Diagnostics) != 0 {
		t.Fatalf("expected no diagnostics, got %#v", result.Diagnostics)
	}
}

func TestAnalyzeModuleAllowsPublicBindingAcrossPackages(t *testing.T) {
	dir := t.TempDir()
	writeModuleFile(t, filepath.Join(dir, "lib.lum"), `
module lib

public answer Int = 7
`)
	writeModuleFile(t, filepath.Join(dir, "main.lum"), `
module app

import lib
import lib/{answer as direct}

def run() Int {
	return lib.answer + direct
}
`)

	mod, err := module.Load(filepath.Join(dir, "main.lum"))
	if err != nil {
		t.Fatalf("Load returned error: %v", err)
	}
	result := AnalyzeModule(mod)
	if len(result.Diagnostics) != 0 {
		t.Fatalf("expected no diagnostics, got %#v", result.Diagnostics)
	}
}

func TestAnalyzePublicBindingRequiresExplicitType(t *testing.T) {
	src := `
public answer = 7
`

	result := Analyze(parseProgram(t, src))
	if len(result.Diagnostics) == 0 {
		t.Fatalf("expected diagnostics, got none")
	}
	if result.Diagnostics[0].Code != "cannot_infer_public_binding_type" {
		t.Fatalf("unexpected diagnostic %#v", result.Diagnostics[0])
	}
}

func TestAnalyzeModuleExtendedImports(t *testing.T) {
	dir := t.TempDir()
	writeModuleFile(t, filepath.Join(dir, "model", "things.lum"), `
module things

interface Named {
	def label() Str
}

class A with Named {
}

impl A {
	def label() Str = "A"
}

class B with Named {
}

impl B {
	def label() Str = "B"
}

object C {
	def bump(value Int) Int = value + 1
	def print(value Int) Int = value + 10
	def printLn(value Int) Int = value + 100
}
`)
	writeModuleFile(t, filepath.Join(dir, "main.lum"), `
module app

import model/things
import model/things/A
import model/things/A as AliasA
import model/things/{B as AliasB, Named}
import model/things/*
import model/things/C/*
import model/things/C/{printLn as printN, print}

def run() Int {
	value A = A()
	aliasA AliasA = AliasA()
	valueB AliasB = AliasB()
	named Named = value
	total Int = C.bump(4) + printN(2) + print(3)
	return total + things.C.bump(5)
}
`)

	mod, err := module.Load(filepath.Join(dir, "main.lum"))
	if err != nil {
		t.Fatalf("Load returned error: %v", err)
	}
	result := AnalyzeModule(mod)
	if len(result.Diagnostics) != 0 {
		t.Fatalf("expected no diagnostics, got %#v", result.Diagnostics)
	}
}

func TestAnalyzeImplicitOSMethods(t *testing.T) {
	src := `
def run() Unit {
	println("hello")
	print("x")
	printf("%s", "y")
	OS.println("z")
}
`

	result := Analyze(parseProgram(t, src))
	if len(result.Diagnostics) != 0 {
		t.Fatalf("expected no diagnostics, got %#v", result.Diagnostics)
	}
}

func TestAnalyzeInterfaceImplementation(t *testing.T) {
	src := `
interface Stringable {
	def show() Str
}

class Good with Stringable {
}

impl Good {
	def init() {
	}

	def show() Str {
		return "ok"
	}
}

class Bad with Stringable {
}

impl Bad {
	def init() {
	}
}
`

	result := Analyze(parseProgram(t, src))
	if len(result.Diagnostics) != 1 {
		t.Fatalf("expected 1 diagnostic, got %#v", result.Diagnostics)
	}
	if result.Diagnostics[0].Code != "interface_not_implemented" {
		t.Fatalf("unexpected diagnostic %#v", result.Diagnostics[0])
	}
}

func TestAnalyzeInterfaceImplementationWithPlainDef(t *testing.T) {
	src := `
interface Stringable {
	def show() Str
}

class Bad with Stringable {
}

impl Bad {
	def show() Str {
		return "ok"
	}
}
`

	result := Analyze(parseProgram(t, src))
	if len(result.Diagnostics) != 0 {
		t.Fatalf("expected no diagnostics, got %#v", result.Diagnostics)
	}
}

func TestAnalyzeInterfaceInheritance(t *testing.T) {
	src := `
interface Hopper {
	def hop() Str
}
interface Jumper {
	def jump(steps Int) Str
}

interface Acrobat with Hopper, Jumper {
}

class Rabbit with Acrobat {
}

impl Rabbit {
	def hop() Str = "hop"
	def jump(steps Int) Str = "jump " + steps
}

def run() Str {
	rabbit = Rabbit()
	return rabbit.hop() + " " + rabbit.jump(3)
}
`

	result := Analyze(parseProgram(t, src))
	if len(result.Diagnostics) != 0 {
		t.Fatalf("expected no diagnostics, got %#v", result.Diagnostics)
	}
}

func TestAnalyzeInterfaceDefaultMethod(t *testing.T) {
	src := `
interface Hopper {
	def hop() Str = "hop"
}

class Rabbit with Hopper {
}
`

	result := Analyze(parseProgram(t, src))
	if len(result.Diagnostics) != 0 {
		t.Fatalf("expected no diagnostics, got %#v", result.Diagnostics)
	}
}

func TestAnalyzeImmutableFieldAssignmentInConstructor(t *testing.T) {
	src := `
class Counter {
	hidden count Int
}

impl Counter {
	def init(count Int) {
		this.count = count
	}

	def read() Int {
		return this.count
	}
}
`

	result := Analyze(parseProgram(t, src))
	if len(result.Diagnostics) != 0 {
		t.Fatalf("expected no diagnostics, got %#v", result.Diagnostics)
	}
}

func TestAnalyzeImmutableFieldAssignmentInInitMethodFails(t *testing.T) {
	src := `
class Counter {
	hidden count Int
}

impl Counter {
	def setup(count Int) {
		this.count = count
	}
}
`

	result := Analyze(parseProgram(t, src))
	if len(result.Diagnostics) != 2 {
		t.Fatalf("expected 2 diagnostics, got %#v", result.Diagnostics)
	}
	if result.Diagnostics[0].Code != "constructor_required" {
		t.Fatalf("unexpected first diagnostic %#v", result.Diagnostics[0])
	}
	if result.Diagnostics[1].Code != "assign_immutable" {
		t.Fatalf("unexpected second diagnostic %#v", result.Diagnostics[1])
	}
}

func TestAnalyzeImmutableFieldAssignmentOutsideConstructor(t *testing.T) {
	src := `
class Counter {
	hidden count Int
}

impl Counter {
	def init(count Int) {
		this.count = count
	}

	def bump() Int {
		this.count = this.count + 1
		return this.count
	}
}
`

	result := Analyze(parseProgram(t, src))
	if len(result.Diagnostics) != 1 {
		t.Fatalf("expected 1 diagnostic, got %#v", result.Diagnostics)
	}
	if result.Diagnostics[0].Code != "assign_immutable" {
		t.Fatalf("unexpected diagnostic %#v", result.Diagnostics[0])
	}
}

func TestAnalyzePrivateAccessOutsideClass(t *testing.T) {
	src := `
class SecretBox {
	hidden value Int
}

impl SecretBox {
	def init(value Int) {
		this.value = value
	}

	hidden def reveal() Int {
		return this.value
	}
}

def run() Int {
	box SecretBox = SecretBox(7)
	x Int = box.value
	return box.reveal()
}
`

	result := Analyze(parseProgram(t, src))
	if len(result.Diagnostics) != 2 {
		t.Fatalf("expected 2 diagnostics, got %#v", result.Diagnostics)
	}
	if result.Diagnostics[0].Code != "private_access" {
		t.Fatalf("unexpected first diagnostic %#v", result.Diagnostics[0])
	}
	if result.Diagnostics[1].Code != "private_access" {
		t.Fatalf("unexpected second diagnostic %#v", result.Diagnostics[1])
	}
}

func TestAnalyzePrivateAccessInsideClass(t *testing.T) {
	src := `
class SecretBox {
	hidden value Int
}

impl SecretBox {
	def init(value Int) {
		this.value = value
	}

	hidden def reveal() Int {
		return this.value
	}

	def expose() Int {
		return this.reveal()
	}
}
`

	result := Analyze(parseProgram(t, src))
	if len(result.Diagnostics) != 0 {
		t.Fatalf("expected no diagnostics, got %#v", result.Diagnostics)
	}
}

func TestAnalyzeMethodOverloadResolution(t *testing.T) {
	src := `
class Counter {
	hidden count Int
}

impl Counter {
	def init(count Int) {
		this.count = count
	}

	def value() Int {
		return this.count
	}

	def add(value Int) Int {
		return this.count + value
	}

	def add(left Int, right Int) Int {
		return left + right
	}
}

def run() Int {
	counter Counter = Counter(2)
	return counter.add(1)
}
`

	result := Analyze(parseProgram(t, src))
	if len(result.Diagnostics) != 0 {
		t.Fatalf("expected no diagnostics, got %#v", result.Diagnostics)
	}
}

func TestAnalyzeNoMatchingMethodOverload(t *testing.T) {
	src := `
class Counter {
	hidden count Int
}

impl Counter {
	def init(count Int) {
		this.count = count
	}

	def add(value Int) Int {
		return this.count + value
	}
}

def run() Int {
	counter Counter = Counter(2)
	return counter.add(true)
}
`

	result := Analyze(parseProgram(t, src))
	if len(result.Diagnostics) != 1 {
		t.Fatalf("expected 1 diagnostic, got %#v", result.Diagnostics)
	}
	if result.Diagnostics[0].Code != "no_matching_overload" {
		t.Fatalf("unexpected diagnostic %#v", result.Diagnostics[0])
	}
}

func TestAnalyzeMethodReferenceWithoutCall(t *testing.T) {
	src := `
class Counter {
	hidden count Int
}

impl Counter {
	def init(count Int) {
		this.count = count
	}

	def add(value Int) Int {
		return this.count + value
	}
}

def run() Int {
	counter Counter = Counter(2)
	f Int = counter.add
	return 0
}
`

	result := Analyze(parseProgram(t, src))
	if len(result.Diagnostics) != 1 {
		t.Fatalf("expected 1 diagnostic, got %#v", result.Diagnostics)
	}
	if result.Diagnostics[0].Code != "invalid_member_access" {
		t.Fatalf("unexpected diagnostic %#v", result.Diagnostics[0])
	}
}

func TestAnalyzeDuplicateConstructorOverload(t *testing.T) {
	src := `
class Counter {
	hidden count Int
}

impl Counter {
	def init(count Int) {
		this.count = count
	}

	def init(value Int) {
		this.count = value
	}
}
`

	result := Analyze(parseProgram(t, src))
	if len(result.Diagnostics) == 0 {
		t.Fatalf("expected duplicate constructor diagnostics, got none")
	}
	if result.Diagnostics[0].Code != "duplicate_constructor" {
		t.Fatalf("unexpected diagnostic %#v", result.Diagnostics[0])
	}
}

func TestAnalyzeImplicitConstructorRequiresMutableOnlyFields(t *testing.T) {
	src := `
class Counter {
	hidden count Int
}
`

	result := Analyze(parseProgram(t, src))
	if len(result.Diagnostics) != 1 {
		t.Fatalf("expected 1 diagnostic, got %#v", result.Diagnostics)
	}
	if result.Diagnostics[0].Code != "constructor_required" {
		t.Fatalf("unexpected diagnostic %#v", result.Diagnostics[0])
	}
}

func TestAnalyzeConstructorMustInitializeImmutableFields(t *testing.T) {
	src := `
class Counter {
	hidden count Int
	hidden seen Bool = ?
}

impl Counter {
	def init() {
		this.seen = false
	}
}
`

	result := Analyze(parseProgram(t, src))
	if len(result.Diagnostics) != 1 {
		t.Fatalf("expected 1 diagnostic, got %#v", result.Diagnostics)
	}
	if result.Diagnostics[0].Code != "uninitialized_field" {
		t.Fatalf("unexpected diagnostic %#v", result.Diagnostics[0])
	}
}

func TestAnalyzeLambdaFunctionValue(t *testing.T) {
	src := `
def run() Int {
	add = (x Int) -> x + 1
	return add(2)
}
`

	result := Analyze(parseProgram(t, src))
	if len(result.Diagnostics) != 0 {
		t.Fatalf("expected no diagnostics, got %#v", result.Diagnostics)
	}
}

func TestAnalyzeBlockLambdaFunctionValue(t *testing.T) {
	src := `
def run() Int {
	add = (x Int) -> {
		y Int = x + 1
		return y
	}
	return add(2)
}
`

	result := Analyze(parseProgram(t, src))
	if len(result.Diagnostics) != 0 {
		t.Fatalf("expected no diagnostics, got %#v", result.Diagnostics)
	}
}

func TestAnalyzeLambdaCannotCaptureVar(t *testing.T) {
	src := `
def run() Int {
	var total Int = 1
	add = (x Int) -> x + total
	return add(2)
}
`

	result := Analyze(parseProgram(t, src))
	if len(result.Diagnostics) != 1 {
		t.Fatalf("expected 1 diagnostic, got %#v", result.Diagnostics)
	}
	if result.Diagnostics[0].Code != "invalid_lambda_capture" {
		t.Fatalf("unexpected diagnostic %#v", result.Diagnostics[0])
	}
}

func TestAnalyzeLambdaCanUseEnclosingGenericType(t *testing.T) {
	src := `
class Box[T] {
	hidden value T
}

impl Box[T] {
	def init(value T) {
		this.value = value
	}

	def transform() T {
		id = (item T) -> item
		return id(this.value)
	}
}
`

	result := Analyze(parseProgram(t, src))
	if len(result.Diagnostics) != 0 {
		t.Fatalf("expected no diagnostics, got %#v", result.Diagnostics)
	}
}

func TestAnalyzeFunctionTypeBindingAndContextualLambda(t *testing.T) {
	src := `
def run() Int {
	add Int -> Int = x -> x + 1
	return add(2)
}
`

	result := Analyze(parseProgram(t, src))
	if len(result.Diagnostics) != 0 {
		t.Fatalf("expected no diagnostics, got %#v", result.Diagnostics)
	}
}

func TestAnalyzeContextualLambdaInFunctionCall(t *testing.T) {
	src := `
def apply(value Int, f Int -> Int) Int {
	return f(value)
}

def run() Int {
	return apply(2, x -> x + 1)
}
`

	result := Analyze(parseProgram(t, src))
	if len(result.Diagnostics) != 0 {
		t.Fatalf("expected no diagnostics, got %#v", result.Diagnostics)
	}
}

func TestAnalyzeGenericFunctionAndMethodCalls(t *testing.T) {
	src := `
def id[T](value T) T = value

class Mapper {
}

impl Mapper {
	def map[X](value Int, fn Int -> X) X {
		fn(value)
	}
}

def run() Str {
	mapper Mapper = Mapper()
	mapped Str = mapper.map(5, (x Int) -> "value=" + x)
	return id(mapped)
}
`

	result := Analyze(parseProgram(t, src))
	if len(result.Diagnostics) != 0 {
		t.Fatalf("expected no diagnostics, got %#v", result.Diagnostics)
	}
}

func TestAnalyzeUntypedLambdaWithoutContextFails(t *testing.T) {
	src := `
def run() Int {
	add = x -> x + 1
	return 0
}
`

	result := Analyze(parseProgram(t, src))
	if len(result.Diagnostics) != 1 {
		t.Fatalf("expected 1 diagnostic, got %#v", result.Diagnostics)
	}
	if result.Diagnostics[0].Code != "invalid_lambda_type" {
		t.Fatalf("unexpected diagnostic %#v", result.Diagnostics[0])
	}
}

func TestAnalyzeEqSupportsClassEquality(t *testing.T) {
	src := `
class Counter with Eq[Counter] {
	hidden count Int
}

impl Counter {
	def init(count Int) {
		this.count = count
	}

	def equals(other Counter) Bool {
		return this.count == other.count
	}
}

def run() Bool {
	left Counter = Counter(1)
	right Counter = Counter(1)
	return left == right
}
`

	result := Analyze(parseProgram(t, src))
	if len(result.Diagnostics) != 0 {
		t.Fatalf("expected no diagnostics, got %#v", result.Diagnostics)
	}
}

func TestAnalyzeClassEqualityRequiresEq(t *testing.T) {
	src := `
class Counter {
	hidden count Int
}

impl Counter {
	def init(count Int) {
		this.count = count
	}
}

def run() Bool {
	left Counter = Counter(1)
	right Counter = Counter(1)
	return left == right
}
`

	result := Analyze(parseProgram(t, src))
	if len(result.Diagnostics) != 1 {
		t.Fatalf("expected 1 diagnostic, got %#v", result.Diagnostics)
	}
	if result.Diagnostics[0].Code != "invalid_binary_operand" {
		t.Fatalf("unexpected diagnostic %#v", result.Diagnostics[0])
	}
}

func TestAnalyzeBuiltinCollectionAndTermInterfaces(t *testing.T) {
	src := `
object Ascending with Ordering[Int] {
	def compare(left Int, right Int) Int = left - right
}

def run() Int {
	items List[Int] = List(1, 2)
	items.append(3)
	items.sort(Ascending)

	values Map[Str, Int] = Map("a" : 1)
	values.put("b", 2)

	seen Set[Int] = Set(1, 2)
	if seen.contains(2) {
		OS.println("ok")
	}

	return items.get(0).getOr(0) + values.get("a").getOr(0)
}
`

	result := Analyze(parseProgram(t, src))
	if len(result.Diagnostics) != 0 {
		t.Fatalf("expected no diagnostics, got %#v", result.Diagnostics)
	}
}

func TestAnalyzeListSortWithOrdering(t *testing.T) {
	src := `
object Descending with Ordering[Int] {
	def compare(left Int, right Int) Int = right - left
}

def run() Int {
	items List[Int] = List(3, 1, 2)
	items.sort(Descending)
	return items.get(0).getOr(0) * 100 + items.get(1).getOr(0) * 10 + items.get(2).getOr(0)
}
`

	result := Analyze(parseProgram(t, src))
	if len(result.Diagnostics) != 0 {
		t.Fatalf("expected no diagnostics, got %#v", result.Diagnostics)
	}
}

func TestAnalyzeListIsEmptyAndRemoveLast(t *testing.T) {
	src := `
def run() Bool {
	items List[Int] = List(1, 2, 3)
	first Bool = items.isEmpty()
	last Option[Int] = items.removeLast()
	nowTwo Int = items.size()
	empty List[Int] = []
	missing Option[Int] = empty.removeLast()
	return !first && last.getOr(0) == 3 && nowTwo == 2 && missing.isEmpty() && empty.isEmpty()
}
`

	result := Analyze(parseProgram(t, src))
	if len(result.Diagnostics) != 0 {
		t.Fatalf("expected no diagnostics, got %#v", result.Diagnostics)
	}
}

func TestAnalyzeListMapFlatMapForEach(t *testing.T) {
	src := `
def run() Int {
	items List[Int] = List(1, 2, 3)
	doubled List[Int] = items.map((item Int) -> item * 2)
	doubled2 List[Int] = items.map(item -> item * 2)
	expanded List[Int] = items.flatMap((item Int) -> List(item, item + 10))
	filtered List[Int] = items.filter((item Int) -> item > 1)
	total Int = items.fold(0, (acc Int, item Int) -> acc + item)
	reduced Option[Int] = items.reduce((left Int, right Int) -> left + right)
	hasBig Bool = items.exists((item Int) -> item > 2)
	allPositive Bool = items.forAll((item Int) -> item > 0)
	doubled.forEach((item Int) -> OS.println(item))
	if hasBig && allPositive {
		return doubled.get(2).getOr(0) + doubled2.get(1).getOr(0) + expanded.get(5).getOr(0) + filtered.size() + total + reduced.getOr(0)
	}
	return 0
}
	`

	result := Analyze(parseProgram(t, src))
	if len(result.Diagnostics) != 0 {
		t.Fatalf("expected no diagnostics, got %#v", result.Diagnostics)
	}
}

func TestAnalyzeListArrayAndRangeZipMethods(t *testing.T) {
	src := `
def run() Int {
	items List[Int] = List(1, 2, 3)
	pairs List[(Int, Str)] = items.zip(List("a", "b"))
	indexed List[(Int, Int)] = items.zipWithIndex()

	values Array[Int] = Array.ofLength(3)
	values[0] := 4
	values[1] := 5
	values[2] := 6
	other Array[Str] = Array.ofLength(2)
	other[0] := "x"
	other[1] := "y"
	valuePairs Array[(Int, Str)] = values.zip(other)
	valueIndexed Array[(Int, Int)] = values.zipWithIndex()

	unwrap firstPair <- pairs.get(0) else {
		0
	}
	unwrap indexedPair <- indexed.get(2) else {
		0
	}
	let (firstLeft Int, firstRight Str) = firstPair
	let (indexedValue Int, indexedPos Int) = indexedPair
	let (arrayLeft Int, arrayRight Str) = valuePairs[1]
	let (arrayIndexedValue Int, arrayIndexedPos Int) = valueIndexed[2]
	var total Int = 0

	for left Int, right Str <- pairs {
		if right == "b" {
			total += left
		}
	}
	for value Int, index Int <- indexed {
		total += value + index
	}
	for left Int, right Str <- valuePairs {
		if right == "y" {
			total += left
		}
	}
	for value Int, index Int <- valueIndexed {
		if index == 2 {
			total += value
		}
	}

	if firstRight == "a" && arrayRight == "y" {
		return firstLeft + indexedValue + indexedPos + arrayLeft + arrayIndexedValue + arrayIndexedPos + pairs.size() + valuePairs.size() + total
	}
	return 0
}
	`

	result := Analyze(parseProgram(t, src))
	if len(result.Diagnostics) != 0 {
		t.Fatalf("expected no diagnostics, got %#v", result.Diagnostics)
	}
}

func TestAnalyzeArrayHigherOrderMethods(t *testing.T) {
	src := `
def run() Int {
	values Array[Int] = Array.ofLength(3)
	values[0] := 4
	values[1] := 5
	values[2] := 6

	mapped Array[Int] = values.map(item -> item * 2)
	hasBig Bool = values.exists(item -> item > 5)
	allPositive Bool = values.forAll(item -> item > 0)
	mapped.forEach(item -> OS.println(item))

	if hasBig && allPositive {
		return mapped[0] + mapped[2] + values.size()
	}
	return 0
}
	`

	result := Analyze(parseProgram(t, src))
	if len(result.Diagnostics) != 0 {
		t.Fatalf("expected no diagnostics, got %#v", result.Diagnostics)
	}
}

func TestAnalyzeSetAndMapHigherOrderMethods(t *testing.T) {
	src := `
def run() Int {
	seen Set[Int] = Set(1, 2, 3)
	doubled Set[Int] = seen.map((item Int) -> item * 2)
	expanded Set[Int] = seen.flatMap((item Int) -> Set(item, item + 10))
	filtered Set[Int] = seen.filter((item Int) -> item > 1)
	setTotal Int = seen.fold(0, (acc Int, item Int) -> acc + item)
	setReduced Option[Int] = seen.reduce((left Int, right Int) -> left + right)
	setHasBig Bool = seen.exists((item Int) -> item > 2)
	setAllPositive Bool = seen.forAll((item Int) -> item > 0)
	seen.forEach((item Int) -> OS.println(item))

	values Map[Str, Int] = Map("a" : 1, "b" : 2)
	mapped List[Int] = values.map((key Str, value Int) -> value * 10)
	mappedValues Map[Str, Int] = values.mapValues((value Int) -> value * 100)
	expandedValues List[Int] = values.flatMap((key Str, value Int) -> List(value, value + 10))
	filteredMap Map[Str, Int] = values.filter((key Str, value Int) -> value > 1)
	mapTotal Int = values.fold(0, (acc Int, key Str, value Int) -> acc + value)
	mapReduced Option[(Str, Int)] = values.reduce((leftKey Str, leftValue Int, rightKey Str, rightValue Int) -> (rightKey, rightValue))
	mapHasB Bool = values.exists((key Str, value Int) -> key == "b")
	mapAllSmall Bool = values.forAll((key Str, value Int) -> value < 3)
	values.forEach((key Str, value Int) -> OS.println(key))

	var total Int = 0
	for item Int <- seen {
		total += item
	}
	for key Str, value Int <- values {
		total += value
	}

	unwrap reducedPair <- mapReduced else {
		0
	}
	let (reducedKey Str, reducedValue Int) = reducedPair
	if expanded.contains(12) && setHasBig && setAllPositive && mapHasB && mapAllSmall {
		if reducedKey == "b" {
			return total + mapped.get(0).getOr(0) + mappedValues["b"].getOr(0) + expandedValues.get(3).getOr(0) + doubled.size() + filtered.size() + setTotal + setReduced.getOr(0) + filteredMap.size() + mapTotal + reducedValue
		}
	}
	return 0
}
	`

	result := Analyze(parseProgram(t, src))
	if len(result.Diagnostics) != 0 {
		t.Fatalf("expected no diagnostics, got %#v", result.Diagnostics)
	}
}

func TestAnalyzeMapFoldInfersAccumulatorLambdaType(t *testing.T) {
	src := `
def run() Int {
	values Map[Str, Int] = Map("a" : 1, "b" : 2)
	total Int = values.fold(0, (acc, key, value) -> acc + value)
	return total
}
	`

	result := Analyze(parseProgram(t, src))
	if len(result.Diagnostics) != 0 {
		t.Fatalf("expected no diagnostics, got %#v", result.Diagnostics)
	}
}

func TestAnalyzeCustomAndCollectionOperators(t *testing.T) {
	src := `
class Vec {
	hidden var items Array[Int]
}

impl Vec {
	def init(left Int, right Int) {
		this.items := Array.ofLength(2)
		this.items[0] := left
		this.items[1] := right
	}

	def [](index Int) Int = items[index]
	def +(other Vec) Vec = Vec(this[0] + other[0], this[1] + other[1])
	def -() Vec = Vec(-this[0], -this[1])
	def :-(other Vec) Vec = Vec(this[0] - other[0], this[1] - other[1])
	def --(other Vec) Vec = Vec(this[0] - other[0] - 1, this[1] - other[1] - 1)
}

def run() Int {
	left Vec = Vec(1, 2)
	right Vec = Vec(3, 4)
	total Vec = left + right
	neg Vec = -total
	diff Vec = total :- left
	trimmed Vec = total -- left

	items List[Int] = List(1, 2)
	items2 List[Int] = items :+ 3
	merged List[Int] = items2 ++ List(4, 5)

	seen Set[Int] = Set(1, 2)
	all Set[Int] = seen ++ Set(3)

	return neg[0] + diff[0] + trimmed[1] + merged[4] + all.size()
}
`

	result := Analyze(parseProgram(t, src))
	if len(result.Diagnostics) != 0 {
		t.Fatalf("expected no diagnostics, got %#v", result.Diagnostics)
	}
}

func TestAnalyzeGenericBounds(t *testing.T) {
	src := `
object Ascending with Ordering[Int] {
	def compare(left Int, right Int) Int = left - right
}

class Box[T with Ordering[T]] {
	value T
}

class Mapper {
}

impl Mapper {
	def pick[X with Ordering[X]](value X) X = value
}

def pick[T with Ordering[T]](value T) T = value
def useBox(value Box[Int]) Int = value.value

def run() Int {
	mapper Mapper = Mapper()
	value Int = pick(3)
	return value + mapper.pick(4)
}
`

	result := Analyze(parseProgram(t, src))
	if len(result.Diagnostics) != 0 {
		t.Fatalf("expected no diagnostics, got %#v", result.Diagnostics)
	}
}

func TestAnalyzeGenericBoundsRejectMissingWitness(t *testing.T) {
	src := `
class Box[T with Ordering[T]] {
	value T
}

def pick[T with Ordering[T]](value T) T = value
def badBox(value Box[Str]) Int = 0

def run() Int {
value Str = pick("x")
	return 0
}
`

	result := Analyze(parseProgram(t, src))
	if len(result.Diagnostics) == 0 {
		t.Fatalf("expected diagnostics for missing Ordering[Str] witness")
	}
	if result.Diagnostics[0].Code != "type_argument_bound" {
		t.Fatalf("unexpected diagnostic %#v", result.Diagnostics[0])
	}
}

func TestAnalyzeRejectDeferredBindingsOutsideClasses(t *testing.T) {
	src := `
value Int = ?

def run() Int {
	local Int = ?
	return 0
}
`

	result := Analyze(parseProgram(t, src))
	if len(result.Diagnostics) != 2 {
		t.Fatalf("expected 2 diagnostics, got %#v", result.Diagnostics)
	}
	for _, diag := range result.Diagnostics {
		if diag.Code != "invalid_deferred" {
			t.Fatalf("unexpected diagnostic %#v", diag)
		}
	}
}

func TestAnalyzeTermPrintlnAnyTypes(t *testing.T) {
	src := `
def run() Int {
	OS.println("count", 10, true)
	return 0
}
`

	result := Analyze(parseProgram(t, src))
	if len(result.Diagnostics) != 0 {
		t.Fatalf("expected no diagnostics, got %#v", result.Diagnostics)
	}
}

func TestAnalyzeOptionConstructorsAndMethods(t *testing.T) {
	src := `
def run() Int {
	found Option[Int] = Some(5)
	missing Option[Int] = None()
	unwrap value <- found else return missing.getOr(7)
	return value
}
`

	result := Analyze(parseProgram(t, src))
	if len(result.Diagnostics) != 0 {
		t.Fatalf("expected no diagnostics, got %#v", result.Diagnostics)
	}
}

func TestAnalyzeStringConcatenation(t *testing.T) {
	src := `
def run() Str {
	count Int = 10
	return "counter " + count
}
`

	result := Analyze(parseProgram(t, src))
	if len(result.Diagnostics) != 0 {
		t.Fatalf("expected no diagnostics, got %#v", result.Diagnostics)
	}
}

func TestAnalyzeIfAndForYieldExpressions(t *testing.T) {
	src := `
def run(values List[Int], flag Bool) Int {
	label Int = if flag {
		1
	} else {
		2
	}
	items List[Int] = for {
		x <- values,
		y <- values
	} yield {
		x + y
	}
	items2 List[Int] = for item <- values yield {
		item + 1
	}
	return label + items.size() + items2.size()
}
`

	result := Analyze(parseProgram(t, src))
	if len(result.Diagnostics) != 0 {
		t.Fatalf("expected no diagnostics, got %#v", result.Diagnostics)
	}
}

func TestAnalyzeForDestructuring(t *testing.T) {
	src := `
def run(rows List[(Int, Str)]) Int {
	var total Int = 0
	for left Int, right Str <- rows {
		if right == "x" {
			total += left
		}
	}
	return total
}
`

	result := Analyze(parseProgram(t, src))
	if len(result.Diagnostics) != 0 {
		t.Fatalf("expected no diagnostics, got %#v", result.Diagnostics)
	}
}

func TestAnalyzeWhileLoop(t *testing.T) {
	src := `
def run(limit Int) Int {
	var total Int = 0
	while total < limit {
		total += 1
	}
	return total
}
`

	result := Analyze(parseProgram(t, src))
	if len(result.Diagnostics) != 0 {
		t.Fatalf("expected no diagnostics, got %#v", result.Diagnostics)
	}
}

func TestAnalyzeForYieldImmutableBindings(t *testing.T) {
	src := `
def run(values List[Int]) List[Int] {
	return for {
		item <- values
		next = item + 1
	} yield {
		next
	}
}
`

	result := Analyze(parseProgram(t, src))
	if len(result.Diagnostics) != 0 {
		t.Fatalf("expected no diagnostics, got %#v", result.Diagnostics)
	}
}

func TestAnalyzeForYieldMutableBindings(t *testing.T) {
	src := `
def run(values List[Int]) List[Int] {
	return for {
		item <- values
		var total = item
	} yield {
		total += 1
		total
	}
}
`

	result := Analyze(parseProgram(t, src))
	if len(result.Diagnostics) != 0 {
		t.Fatalf("expected no diagnostics, got %#v", result.Diagnostics)
	}
}

func TestAnalyzeRejectsUselessExpressionStatement(t *testing.T) {
	src := `
def run() Int {
	value = 1
	value
	return value
}
`

	result := Analyze(parseProgram(t, src))
	if len(result.Diagnostics) == 0 {
		t.Fatalf("expected diagnostics, got none")
	}
	if result.Diagnostics[0].Code != "useless_expression" {
		t.Fatalf("expected useless_expression, got %#v", result.Diagnostics[0])
	}
}

func TestAnalyzeIsExpression(t *testing.T) {
	src := `
class Counter {
}

def run() Bool {
	value = Counter()
	return value is Counter
}
`

	result := Analyze(parseProgram(t, src))
	if len(result.Diagnostics) != 0 {
		t.Fatalf("expected no diagnostics, got %#v", result.Diagnostics)
	}
}

func TestAnalyzeNamedCallArguments(t *testing.T) {
	src := `
class Counter {
}

impl Counter {
	def set(value Int, label Str) Int {
		return value
	}
}

def doSomething(a Str, b Int) Int {
	return b
}

def run() Int {
	counter = Counter()
	left Int = doSomething(b = 5, a = "crap")
	right Int = counter.set(label = "ok", value = 7)
	return left + right
}
`

	result := Analyze(parseProgram(t, src))
	if len(result.Diagnostics) != 0 {
		t.Fatalf("expected no diagnostics, got %#v", result.Diagnostics)
	}
}

func TestAnalyzeUnitLambda(t *testing.T) {
	src := `
def run() Int {
	action () -> Unit = () -> OS.println("hi")
	action()
	return 0
}
`

	result := Analyze(parseProgram(t, src))
	if len(result.Diagnostics) != 0 {
		t.Fatalf("expected no diagnostics, got %#v", result.Diagnostics)
	}
}

func TestAnalyzeUnitLiteralAndGroupedExpr(t *testing.T) {
	src := `
def run() Bool {
	unit Unit = ()
	value Int = (1)
	return value == 1
}
`

	result := Analyze(parseProgram(t, src))
	if len(result.Diagnostics) != 0 {
		t.Fatalf("expected no diagnostics, got %#v", result.Diagnostics)
	}
}

func TestAnalyzeExplicitReturnValueInUnitFunctionFails(t *testing.T) {
	src := `
def run() Unit {
	return 1
}
`

	result := Analyze(parseProgram(t, src))
	if len(result.Diagnostics) != 1 {
		t.Fatalf("expected 1 diagnostic, got %#v", result.Diagnostics)
	}
	if result.Diagnostics[0].Code != "invalid_return_type" {
		t.Fatalf("expected invalid_return_type, got %#v", result.Diagnostics[0])
	}
}

func TestAnalyzeZeroArgFunctionBindingSugar(t *testing.T) {
	src := `
def run() Int {
	action () -> Int = 1
	return action()
}
`

	result := Analyze(parseProgram(t, src))
	if len(result.Diagnostics) != 0 {
		t.Fatalf("expected no diagnostics, got %#v", result.Diagnostics)
	}
}

func TestAnalyzeFunctionImplicitReturn(t *testing.T) {
	src := `
def suffix(value Str) Str {
	value + "!"
}

def run() Str {
	suffix("hello")
}
`

	result := Analyze(parseProgram(t, src))
	if len(result.Diagnostics) != 0 {
		t.Fatalf("expected no diagnostics, got %#v", result.Diagnostics)
	}
}

func TestAnalyzeLocalFunctionStmt(t *testing.T) {
	src := `
def run() Int {
	boost Int = 2
	def add(value Int) Int = value + boost
	add(5)
}
`

	result := Analyze(parseProgram(t, src))
	if len(result.Diagnostics) != 0 {
		t.Fatalf("expected no diagnostics, got %#v", result.Diagnostics)
	}
}

func TestAnalyzeVariadicFunction(t *testing.T) {
	src := `
def sum(values Int...) Int {
	var total Int = 0
	for value <- values {
		total += value
	}
	total
}

def run() Int {
	sum(1, 2, 3)
}
`

	result := Analyze(parseProgram(t, src))
	if len(result.Diagnostics) != 0 {
		t.Fatalf("expected no diagnostics, got %#v", result.Diagnostics)
	}
}

func TestAnalyzeMultiAssignmentStmt(t *testing.T) {
	src := `
def run() Int {
	var a Int = 0
	var b Str = ""
	a, b := 1, "ok"
	a
}
`

	result := Analyze(parseProgram(t, src))
	if len(result.Diagnostics) != 0 {
		t.Fatalf("expected no diagnostics, got %#v", result.Diagnostics)
	}
}

func TestAnalyzeArrayConstructorAndSize(t *testing.T) {
	src := `
def run() Int {
	values Array[Int] = Array.ofLength(5)
	return values.size()
}
`

	result := Analyze(parseProgram(t, src))
	if len(result.Diagnostics) != 0 {
		t.Fatalf("expected no diagnostics, got %#v", result.Diagnostics)
	}
}

func TestAnalyzeInvalidArrayConstructor(t *testing.T) {
	src := `
def run() Array[Int] {
	return Array(1, "x")
}
`

	result := Analyze(parseProgram(t, src))
	if len(result.Diagnostics) == 0 {
		t.Fatalf("expected diagnostics, got none")
	}
	if result.Diagnostics[0].Code != "type_mismatch" {
		t.Fatalf("unexpected diagnostic %#v", result.Diagnostics[0])
	}
}

func TestAnalyzeIfOptionBinding(t *testing.T) {
	src := `
def run(value Option[Int]) Int {
	if let Some(item) = value {
		return item
	}
	return 0
}
`

	result := Analyze(parseProgram(t, src))
	if len(result.Diagnostics) != 0 {
		t.Fatalf("expected no diagnostics, got %#v", result.Diagnostics)
	}
}

func TestAnalyzeIfOptionBindingRejectsNonOption(t *testing.T) {
	src := `
def run(value Int) Int {
	if let Some(item) = value {
		return item
	}
	return 0
}
`

	result := Analyze(parseProgram(t, src))
	if len(result.Diagnostics) == 0 {
		t.Fatalf("expected diagnostics, got none")
	}
	if result.Diagnostics[0].Code != "invalid_match_pattern" {
		t.Fatalf("unexpected diagnostic %#v", result.Diagnostics[0])
	}
}

func TestAnalyzeIfOptionDestructuring(t *testing.T) {
	src := `
def run(value Option[(Int, Str, Bool)]) Str {
	if let Some(item) = value {
		let (_, name, _) = item
		return name
	}
	return "missing"
}
`

	result := Analyze(parseProgram(t, src))
	if len(result.Diagnostics) != 0 {
		t.Fatalf("expected no diagnostics, got %#v", result.Diagnostics)
	}
}

func TestAnalyzeUnwrapStmtOptionResultEither(t *testing.T) {
	src := `
def plusOneOption(value Option[Int]) Option[Int] {
	unwrap item <- value
	return Some(item + 1)
}

def plusOneResult(value Result[Int, Str]) Result[Int, Str] {
	unwrap item <- value
	return Ok(item + 1)
}

def plusOneEither(value Either[Str, Int]) Either[Str, Int] {
	unwrap item <- value
	return Right(item + 1)
}

def twoEithers(value Either[Str, Int], value2 Either[Str, Str]) Either[Str, Int] {
	unwrap item <- value
	unwrap size <- value2.map((s Str) -> s.size())
	return Right(item + size)
}
`

	result := Analyze(parseProgram(t, src))
	if len(result.Diagnostics) != 0 {
		t.Fatalf("expected no diagnostics, got %#v", result.Diagnostics)
	}
}

func TestAnalyzeUnwrapStmtRejectsIncompatibleReturn(t *testing.T) {
	src := `
def run(value Result[Int, Str]) Option[Int] {
	unwrap item <- value
	return Some(item)
}
`

	result := Analyze(parseProgram(t, src))
	if len(result.Diagnostics) == 0 {
		t.Fatalf("expected diagnostics, got none")
	}
	if result.Diagnostics[0].Code != "invalid_unwrap" {
		t.Fatalf("unexpected diagnostic %#v", result.Diagnostics[0])
	}
}

func TestAnalyzeGuardStmt(t *testing.T) {
	src := `
def run(value Option[Int]) Result[Int, Str] {
	unwrap item <- value else {
		Err("missing")
	}
	return Ok(item + 1)
}
`

	result := Analyze(parseProgram(t, src))
	if len(result.Diagnostics) != 0 {
		t.Fatalf("expected no diagnostics, got %#v", result.Diagnostics)
	}
}

func TestAnalyzeGuardStmtRejectsWrongGuardType(t *testing.T) {
	src := `
def run(value Option[Int]) Result[Int, Str] {
	unwrap item <- value else {
		Some(0)
	}
	return Ok(item + 1)
}
`

	result := Analyze(parseProgram(t, src))
	if len(result.Diagnostics) == 0 {
		t.Fatalf("expected diagnostics, got none")
	}
	if result.Diagnostics[0].Code != "invalid_unwrap" {
		t.Fatalf("unexpected diagnostic %#v", result.Diagnostics[0])
	}
}

func TestAnalyzeGuardBlockStmt(t *testing.T) {
	src := `
def run(left Option[Int], right Option[Int]) Result[Int, Str] {
	unwrap {
		a <- left
		b <- right
	} else {
		Err("missing")
	}
	return Ok(a + b)
}
`

	result := Analyze(parseProgram(t, src))
	if len(result.Diagnostics) != 0 {
		t.Fatalf("expected no diagnostics, got %#v", result.Diagnostics)
	}
}

func TestAnalyzeUnwrapBlockStmt(t *testing.T) {
	src := `
def run(left Option[Int], right Option[Int]) Option[Int] {
	unwrap {
		a <- left
		b <- right
	}
	return Some(a + b)
}
`

	result := Analyze(parseProgram(t, src))
	if len(result.Diagnostics) != 0 {
		t.Fatalf("expected no diagnostics, got %#v", result.Diagnostics)
	}
}

func TestAnalyzeGuardBlockStmtRejectsWrongElseType(t *testing.T) {
	src := `
def run(left Option[Int]) Result[Int, Str] {
	unwrap {
		a <- left
	} else {
		Some(0)
	}
	return Ok(a)
}
`

	result := Analyze(parseProgram(t, src))
	if len(result.Diagnostics) == 0 {
		t.Fatalf("expected diagnostics, got none")
	}
	if result.Diagnostics[0].Code != "invalid_unwrap" {
		t.Fatalf("unexpected diagnostic %#v", result.Diagnostics[0])
	}
}

func TestAnalyzeEnumMethodsUseTopLevelImpl(t *testing.T) {
	src := `
enum Outcome {
	tag Str

	case Left {
		value Str
		tag = "left"
	}

	case Right {
		value Int
		tag = "right"
	}
}

impl Outcome {
	def describe() Str = match this {
		case Left(value) => value
		case Right(value) => "num " + value
	}
}

def run() Str {
	left Outcome = Outcome.Left("bad")
	right Outcome = Outcome.Right(7)
	return left.describe() + "-" + right.describe()
}
`

	result := Analyze(parseProgram(t, src))
	if len(result.Diagnostics) != 0 {
		t.Fatalf("expected no diagnostics, got %#v", result.Diagnostics)
	}
}

func TestAnalyzeMapIndexReturnsOption(t *testing.T) {
	src := `
def run() Bool {
	entries Map[Str, Int] = Map("a": 1)
	present Option[Int] = entries["a"]
	missing Option[Int] = entries["z"]
	return present.isSet() && missing.isEmpty()
}
`

	result := Analyze(parseProgram(t, src))
	if len(result.Diagnostics) != 0 {
		t.Fatalf("expected no diagnostics, got %#v", result.Diagnostics)
	}
}

func TestAnalyzePartialMatchAndPlaceholderMatchIf(t *testing.T) {
	src := `
enum MaybeInt {
	case NoneX
	case SomeX {
		value Int
	}
}

def run() Bool {
	values List[Int] = List(1, 6, 3)
	ifMapped List[Int] = values.map(if _ > 5 then 10 else 8)
	options List[MaybeInt] = List(MaybeInt.SomeX(1), MaybeInt.NoneX, MaybeInt.SomeX(3))
	matchMapped List[Int] = options.map(match {
	case SomeX(x) => x + 1
	case NoneX => 0
	})
	partialMapped = options.map(partial {
	case SomeX(x) => x + 1
	})
	unwrap firstPartial <- partialMapped.get(0) else {
		false
	}
	unwrap secondPartial <- partialMapped.get(1) else {
		false
	}
	return ifMapped.get(1).getOr(0) == 10 &&
		matchMapped.get(0).getOr(0) == 2 &&
		firstPartial.getOr(0) == 2 &&
		secondPartial.isEmpty()
}
`

	result := Analyze(parseProgram(t, src))
	if len(result.Diagnostics) != 0 {
		t.Fatalf("expected no diagnostics, got %#v", result.Diagnostics)
	}
}

func TestAnalyzeTrailingBlockLambda(t *testing.T) {
	src := `
def run() Bool {
	values List[Int] = [1, 2, 3]
	mapped List[Int] = values.map { x -> x + 1 }
	return mapped.get(1).getOr(0) == 3
}
`

	result := Analyze(parseProgram(t, src))
	if len(result.Diagnostics) != 0 {
		t.Fatalf("expected no diagnostics, got %#v", result.Diagnostics)
	}
}

func TestAnalyzeAnnotatedPartialMatchMap(t *testing.T) {
	src := `
enum MaybeInt {
	case NoneX
	case SomeX {
		value Int
	}
}

def run() Bool {
	options List[MaybeInt] = List(MaybeInt.SomeX(1), MaybeInt.NoneX, MaybeInt.SomeX(3))
	partialMapped List[Option[Int]] = options.map(partial {
	case SomeX(x) => x + 1
	})
	return partialMapped.get(1).getOr(None()).isEmpty()
}
`

	result := Analyze(parseProgram(t, src))
	if len(result.Diagnostics) != 0 {
		t.Fatalf("expected no diagnostics, got %#v", result.Diagnostics)
	}
}

func TestAnalyzePartialMethod(t *testing.T) {
	src := `
class Classifier {
}

impl Classifier {
	partial classify(value Int) Int {
	case 3 => 5
	case 4 => 0
	}
}

def run() Bool {
	classifier = Classifier()
	first Option[Int] = classifier.classify(3)
	second Option[Int] = classifier.classify(8)
	return first.getOr(0) == 5 && second.isEmpty()
}
`

	result := Analyze(parseProgram(t, src))
	if len(result.Diagnostics) != 0 {
		t.Fatalf("expected no diagnostics, got %#v", result.Diagnostics)
	}
}

func TestAnalyzeUnwrapStmtRejectsNonUnwrappable(t *testing.T) {
	src := `
def run(value Int) Option[Int] {
	unwrap item <- value
	return Some(item)
}
`

	result := Analyze(parseProgram(t, src))
	if len(result.Diagnostics) == 0 {
		t.Fatalf("expected diagnostics, got none")
	}
	if result.Diagnostics[0].Code != "invalid_unwrap" {
		t.Fatalf("unexpected diagnostic %#v", result.Diagnostics[0])
	}
}

func TestAnalyzeStrSize(t *testing.T) {
	src := `
def run(text Str) Int {
	return text.size()
}
`

	result := Analyze(parseProgram(t, src))
	if len(result.Diagnostics) != 0 {
		t.Fatalf("expected no diagnostics, got %#v", result.Diagnostics)
	}
}

func TestAnalyzeStrSplitAndIndexOf(t *testing.T) {
	src := `
def run(text Str) Int {
	parts Array[Str] = text.split(" ")
	return parts.size() + text.indexOf("lo")
}
`

	result := Analyze(parseProgram(t, src))
	if len(result.Diagnostics) != 0 {
		t.Fatalf("expected no diagnostics, got %#v", result.Diagnostics)
	}
}

func TestAnalyzeNestedBlockExpressions(t *testing.T) {
	src := `
def run() Int {
	a1 Int = {
		1 + 7
	}
	{
		OS.println("xxx")
	}
	var v Int = {
		a Int = 5
		{
			a + 1
		}
	}
	return a1 + v
}
`

	result := Analyze(parseProgram(t, src))
	if len(result.Diagnostics) != 0 {
		t.Fatalf("expected no diagnostics, got %#v", result.Diagnostics)
	}
}

func TestAnalyzeNestedBlockValueStatements(t *testing.T) {
	src := `
def run() Int {
	fromIf Int = {
		if false {
			10
		} else {
			20
		}
	}
	fromYield List[Int] = {
		for item <- [1, 2, 3] yield {
			if item % 2 == 0 {
				item * 10
			} else {
				item
			}
		}
	}
	return fromIf + fromYield.get(1).getOr(0)
}
`

	result := Analyze(parseProgram(t, src))
	if len(result.Diagnostics) != 0 {
		t.Fatalf("expected no diagnostics, got %#v", result.Diagnostics)
	}
}

func TestAnalyzePlaceholderLambdaShorthand(t *testing.T) {
	src := `
def applyTwice(f (Int) -> Int, value Int) Int = f(f(value))

def run() Int {
	inc (Int) -> Int = _ + 1
	items List[Int] = List(1, 2, 3)
	mapped List[Int] = items.map(_ + 1)
	opt Option[Int] = Some(5)
	mappedOpt Option[Int] = opt.map(_ -> 1)
	return applyTwice(inc, mapped.get(0).getOr(0))
}
`

	result := Analyze(parseProgram(t, src))
	if len(result.Diagnostics) != 0 {
		t.Fatalf("expected no diagnostics, got %#v", result.Diagnostics)
	}
}

func TestAnalyzeTupleDestructuringLambdas(t *testing.T) {
	src := `
def run() Int {
	pairs List[(Str, Int)] = List(("a", 1), ("bb", 2))
	pairMapped List[Int] = pairs.map((key, value) -> key.size() + value)
	pairKeys List[Str] = pairs.map((key, _) -> key)
	pairIgnored List[Int] = pairs.map((_, value) -> value * 2)
	tuple4s List[(Int, Int, Int, Int)] = List((1, 2, 3, 4), (4, 5, 6, 7))
	tuple4Mapped List[Int] = tuple4s.map((first, _, third, _) -> first + third)
	entries Map[Str, Int] = Map("a": 1, "bbb": 2)
	mapMapped List[Int] = entries.map((key, value) -> key.size() + value)
	return pairMapped.get(0).getOr(0) +
		pairMapped.get(1).getOr(0) +
		pairKeys.get(1).getOr("").size() +
		mapMapped.get(1).getOr(0) +
		pairIgnored.get(1).getOr(0) +
		tuple4Mapped.get(1).getOr(0)
}
`

	result := Analyze(parseProgram(t, src))
	if len(result.Diagnostics) != 0 {
		t.Fatalf("expected no diagnostics, got %#v", result.Diagnostics)
	}
}
