package interpreter

import (
	"io"
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

func TestCallFunction(t *testing.T) {
	src := `
def add(a Int, b Int) Int {
	return a + b
}

def run(input Int) Int {
	total Int = add(input, 2)
	var copy Int = total
	copy += 3

	if copy > 10 {
		return copy
	}

	return total
}
`

	in := New(parseProgram(t, src))
	value, err := in.Call("run", int64(5))
	if err != nil {
		t.Fatalf("Call returned error: %v", err)
	}
	if value != int64(7) {
		t.Fatalf("expected 7, got %#v", value)
	}
}

func TestModulePublicFunctionAndBinding(t *testing.T) {
	dir := t.TempDir()
	writeModuleFile(t, filepath.Join(dir, "lib.lum"), `
module lib

public def exportedValue() Int = 7
public answer Int = 5
`)
	writeModuleFile(t, filepath.Join(dir, "main.lum"), `
module app

import lib
import lib/{answer as direct}

def run() Int {
	return lib.exportedValue() + lib.answer + direct
}
`)

	mod, err := module.Load(filepath.Join(dir, "main.lum"))
	if err != nil {
		t.Fatalf("Load returned error: %v", err)
	}
	in := NewModule(mod)
	value, err := in.Call("run")
	if err != nil {
		t.Fatalf("Call returned error: %v", err)
	}
	if value != int64(17) {
		t.Fatalf("expected 17, got %#v", value)
	}
}

func TestModuleObjectMethodImports(t *testing.T) {
	dir := t.TempDir()
	writeModuleFile(t, filepath.Join(dir, "lib.lum"), `
module lib

object Tools {
	def inc(value Int) Int = value + 1
	def twice(value Int) Int = value * 2
}
`)
	writeModuleFile(t, filepath.Join(dir, "main.lum"), `
module app

import lib/Tools/*
import lib/Tools/{twice as double}

def run() Int {
	return inc(4) + double(5)
}
`)

	mod, err := module.Load(filepath.Join(dir, "main.lum"))
	if err != nil {
		t.Fatalf("Load returned error: %v", err)
	}
	in := NewModule(mod)
	value, err := in.Call("run")
	if err != nil {
		t.Fatalf("Call returned error: %v", err)
	}
	if value != int64(15) {
		t.Fatalf("expected 15, got %#v", value)
	}
}

func TestImplicitOSMethods(t *testing.T) {
	src := `
def run() Unit {
	println("hello")
	print("x")
	printf("%s", "y")
	OS.println("z")
}
`

	in := New(parseProgram(t, src))
	if _, err := in.Call("run"); err != nil {
		t.Fatalf("Call returned error: %v", err)
	}
}

func TestOSPrinters(t *testing.T) {
	src := `
def run() Unit {
	OS.print("out")
	OS.stdout.println(" line")
	OS.printf(" %s %d", "fmt", 7)
	OS.stderr.println("err line")
	OS.stderr.printf("err %s %d", "fmt", 3)
	OS.println("legacy")
}
`

	oldStdout := os.Stdout
	oldStderr := os.Stderr
	stdoutReader, stdoutWriter, err := os.Pipe()
	if err != nil {
		t.Fatalf("Pipe returned error: %v", err)
	}
	stderrReader, stderrWriter, err := os.Pipe()
	if err != nil {
		t.Fatalf("Pipe returned error: %v", err)
	}
	os.Stdout = stdoutWriter
	os.Stderr = stderrWriter
	defer func() {
		os.Stdout = oldStdout
		os.Stderr = oldStderr
	}()

	in := New(parseProgram(t, src))
	if _, err := in.Call("run"); err != nil {
		t.Fatalf("Call returned error: %v", err)
	}
	_ = stdoutWriter.Close()
	_ = stderrWriter.Close()

	stdoutBytes, err := io.ReadAll(stdoutReader)
	if err != nil {
		t.Fatalf("ReadAll stdout returned error: %v", err)
	}
	stderrBytes, err := io.ReadAll(stderrReader)
	if err != nil {
		t.Fatalf("ReadAll stderr returned error: %v", err)
	}
	if string(stdoutBytes) != "out line\n fmt 7legacy\n" {
		t.Fatalf("unexpected stdout %q", string(stdoutBytes))
	}
	if string(stderrBytes) != "err line\nerr fmt 3" {
		t.Fatalf("unexpected stderr %q", string(stderrBytes))
	}
}

func TestOSPanic(t *testing.T) {
	src := `
def run() Unit {
	OS.panic("boom ", 7)
}
`

	in := New(parseProgram(t, src))
	_, err := in.Call("run")
	if err == nil {
		t.Fatalf("expected panic error, got nil")
	}
	if err.Error() != "panic: boom 7 at 3:2" {
		t.Fatalf("unexpected panic error %q", err.Error())
	}
}

func TestForLoops(t *testing.T) {
	src := `
def run() Int {
	var total Int = 0

	for item <- [1, 2, 3] {
		total += item
	}

	for step <- [1, 3] {
		total += step
	}

	return total
}
`

	in := New(parseProgram(t, src))
	value, err := in.Call("run")
	if err != nil {
		t.Fatalf("Call returned error: %v", err)
	}
	if value != int64(10) {
		t.Fatalf("expected 10, got %#v", value)
	}
}

func TestForRangeLoops(t *testing.T) {
	src := `
def run() Int {
	var total Int = 0

	for item <- Range(1, 4) {
		total += item
	}

	for item <- Range(5, 1) {
		total += item
	}

	return total
}
`

	in := New(parseProgram(t, src))
	value, err := in.Call("run")
	if err != nil {
		t.Fatalf("Call returned error: %v", err)
	}
	if value != int64(20) {
		t.Fatalf("expected 20, got %#v", value)
	}
}

func TestPrivateFieldInferenceInClassAndObject(t *testing.T) {
	src := `
class Box {
	hidden count = 2
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
	def greet(times Int) Str = this.hello + " " + times
}

def run() Int {
	box = Box()
	box.bump()
	box.bump()
	if Greeter.greet(3) == "Hello 3" {
		return box.value()
	}
	return 0
}
`

	in := New(parseProgram(t, src))
	value, err := in.Call("run")
	if err != nil {
		t.Fatalf("Call returned error: %v", err)
	}
	if value != int64(4) {
		t.Fatalf("expected 4, got %#v", value)
	}
}

func TestYieldLoops(t *testing.T) {
	src := `
def run() Int {
	var total Int = 0

	for {
		left <- [1, 2],
		right <- [3, 4]
	} yield {
		total += left + right
		left + right
	}

	return total
}
`

	in := New(parseProgram(t, src))
	value, err := in.Call("run")
	if err != nil {
		t.Fatalf("Call returned error: %v", err)
	}
	if value != int64(20) {
		t.Fatalf("expected 20, got %#v", value)
	}
}

func TestIndexing(t *testing.T) {
	src := `
def run() Int {
	values = [1, 2, 3]
	values[1] := values[0] + 4
	return values[1]
}
`

	in := New(parseProgram(t, src))
	value, err := in.Call("run")
	if err != nil {
		t.Fatalf("Call returned error: %v", err)
	}
	if value != int64(5) {
		t.Fatalf("expected 5, got %#v", value)
	}
}

func TestListArrayAndRangeZipMethods(t *testing.T) {
	src := `
def run() Int {
	items = List(1, 2, 3)
	pairs = items.zip(List("a", "b"))
	indexed = items.zipWithIndex()

	values = Array.ofLength(3)
	values[0] := 4
	values[1] := 5
	values[2] := 6
	other = Array.ofLength(2)
	other[0] := "x"
	other[1] := "y"
	valuePairs = values.zip(other)
	valueIndexed = values.zipWithIndex()

	unwrap firstPair <- pairs.get(0) else {
		0
	}
	unwrap indexedPair <- indexed.get(2) else {
		0
	}
	let (firstLeft, firstRight) = firstPair
	let (indexedValue, indexedPos) = indexedPair
	let (arrayLeft, arrayRight) = valuePairs[1]
	let (arrayIndexedValue, arrayIndexedPos) = valueIndexed[2]
	var total = 0

	for left, right <- pairs {
		if right == "b" {
			total += left
		}
	}
	for value, index <- indexed {
		total += value + index
	}
	for left, right <- valuePairs {
		if right == "y" {
			total += left
		}
	}
	for value, index <- valueIndexed {
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

	in := New(parseProgram(t, src))
	value, err := in.Call("run")
	if err != nil {
		t.Fatalf("Call returned error: %v", err)
	}
	if value != int64(45) {
		t.Fatalf("expected 45, got %#v", value)
	}
}

func TestArrayHigherOrderMethods(t *testing.T) {
	src := `
def run() Int {
	values = Array.ofLength(3)
	values[0] := 4
	values[1] := 5
	values[2] := 6

	mapped = values.map(item -> item * 2)
	hasBig = values.exists(item -> item > 5)
	allPositive = values.forAll(item -> item > 0)
	mapped.forEach(item -> OS.println("array " + item))

	if hasBig && allPositive {
		return mapped[0] * 100 + mapped[2] * 10 + values.size()
	}
	return 0
}
`

	in := New(parseProgram(t, src))
	value, err := in.Call("run")
	if err != nil {
		t.Fatalf("Call returned error: %v", err)
	}
	if value != int64(923) {
		t.Fatalf("expected 923, got %#v", value)
	}
}

func TestClassesAndMethods(t *testing.T) {
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
	counter.inc()
	return counter.inc()
}
`

	in := New(parseProgram(t, src))
	value, err := in.Call("run")
	if err != nil {
		t.Fatalf("Call returned error: %v", err)
	}
	if value != int64(3) {
		t.Fatalf("expected 3, got %#v", value)
	}
}

func TestMethodOverloadDispatch(t *testing.T) {
	src := `
class Adder {
}

impl Adder {
	def add(value Int) Int {
		return value + 1
	}

	def add(value Str) Int {
		return 99
	}
}

def run() Int {
	adder Adder = Adder()
	return adder.add("hehe")
}
`

	in := New(parseProgram(t, src))
	value, err := in.Call("run")
	if err != nil {
		t.Fatalf("Call returned error: %v", err)
	}
	if value != int64(99) {
		t.Fatalf("expected 99, got %#v", value)
	}
}

func TestMethodWithoutReturnTypeDoesNotImplicitlyReturn(t *testing.T) {
	src := `
class Counter {
	hidden var count Int
}

impl Counter {
	def init(count Int) {
		this.count = count
	}

	def touch() {
		this.count + 1
	}
}

def run() Bool {
	counter Counter = Counter(1)
	return counter.touch() == 2
}
`

	in := New(parseProgram(t, src))
	value, err := in.Call("run")
	if err != nil {
		t.Fatalf("Call returned error: %v", err)
	}
	if value != false {
		t.Fatalf("expected false, got %#v", value)
	}
}

func TestExpressionBodiedMethod(t *testing.T) {
	src := `
class Counter {
}

impl Counter {
	def value() Int = 7
}

def run() Int {
	counter Counter = Counter()
	counter.value()
}
`

	in := New(parseProgram(t, src))
	value, err := in.Call("run")
	if err != nil {
		t.Fatalf("Call returned error: %v", err)
	}
	if value != int64(7) {
		t.Fatalf("expected 7, got %#v", value)
	}
}

func TestForDestructuring(t *testing.T) {
	src := `
def run() Int {
	var total Int = 0

	for left Int, right Str <- [(1, "x"), (2, "y"), (3, "x")] {
		if right == "x" {
			total += left
		}
	}

	return total
}
`

	in := New(parseProgram(t, src))
	value, err := in.Call("run")
	if err != nil {
		t.Fatalf("Call returned error: %v", err)
	}
	if value != int64(4) {
		t.Fatalf("expected 4, got %#v", value)
	}
}

func TestMatchStmt(t *testing.T) {
	src := `
enum OptionX[T] {
	case NoneX
	case SomeX {
		value T
	}
}

def run() Int {
	value OptionX[Int] = OptionX.SomeX(7)
	var total Int = 0

	match value {
		case SomeX(item) => {
			total += item
		}
		case OptionX.NoneX => {
			total += 100
		}
	}

	pair = (1, 2)
	match pair {
		case (left, right) => {
			total += left + right
		}
	}

	return total
}
`

	in := New(parseProgram(t, src))
	value, err := in.Call("run")
	if err != nil {
		t.Fatalf("Call returned error: %v", err)
	}
	if value != int64(10) {
		t.Fatalf("expected 10, got %#v", value)
	}
}

func TestMatchTypePattern(t *testing.T) {
	src := `
interface WorkerLike {
	def doWork() Int
}

class Worker with WorkerLike {
}

impl Worker {
	def doWork() Int = 7
}

class Other with WorkerLike {
}

impl Other {
	def doWork() Int = 3
}

def run() Int {
	value WorkerLike = Worker()

	return match value {
		case worker Worker => worker.doWork()
		case _ Other => 100
		case _ => 0
	}
}
`

	in := New(parseProgram(t, src))
	value, err := in.Call("run")
	if err != nil {
		t.Fatalf("Call returned error: %v", err)
	}
	if value != int64(7) {
		t.Fatalf("expected 7, got %#v", value)
	}
}

func TestMatchErasedGenericTypePattern(t *testing.T) {
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

def run() Int {
	return describe(OptionX.SomeX(7)) * 10 + describeBox(Box(5))
}
`

	in := New(parseProgram(t, src))
	value, err := in.Call("run")
	if err != nil {
		t.Fatalf("Call returned error: %v", err)
	}
	if value != int64(12) {
		t.Fatalf("expected 12, got %#v", value)
	}
}

func TestMatchGuard(t *testing.T) {
	src := `
enum OptionX[T] {
	case NoneX
	case SomeX {
		value T
	}
}

def describe(value OptionX[Int]) Int =
	match value {
		case SomeX(x) if x > 10 => x
		case SomeX(_) => 10
		case OptionX.NoneX => 0
	}

def run() Int {
	return describe(OptionX.SomeX(12)) * 100 + describe(OptionX.SomeX(3)) * 10 + describe(OptionX.NoneX())
}
`

	in := New(parseProgram(t, src))
	value, err := in.Call("run")
	if err != nil {
		t.Fatalf("Call returned error: %v", err)
	}
	if value != int64(1300) {
		t.Fatalf("expected 1300, got %#v", value)
	}
}

func TestMatchClassExtractor(t *testing.T) {
	src := `
class PairBox {
	left Int
	right Int
}

def run() Int {
	value PairBox = PairBox(4, 9)
	return match value {
		case PairBox(left, right) => left + right
	}
}
`

	in := New(parseProgram(t, src))
	value, err := in.Call("run")
	if err != nil {
		t.Fatalf("Call returned error: %v", err)
	}
	if value != int64(13) {
		t.Fatalf("expected 13, got %#v", value)
	}
}

func TestMatchGenericEnumAndClassExtractor(t *testing.T) {
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

def run() Int {
	return unwrapSome(OptionX.SomeX(7)) * 10 + unwrapBox(Box(5))
}
`

	in := New(parseProgram(t, src))
	value, err := in.Call("run")
	if err != nil {
		t.Fatalf("Call returned error: %v", err)
	}
	if value != int64(87) {
		t.Fatalf("expected 87, got %#v", value)
	}
}

func TestMatchRecordExtractor(t *testing.T) {
	src := `
record Amount {
	count Int
	label Str
}

def run() Int {
	value Amount = Amount(42, "hello")
	return match value {
		case Amount(count, label) => count
	}
}
`

	in := New(parseProgram(t, src))
	value, err := in.Call("run")
	if err != nil {
		t.Fatalf("Call returned error: %v", err)
	}
	if value != int64(42) {
		t.Fatalf("expected 42, got %#v", value)
	}
}

func TestMatchNestedPatterns(t *testing.T) {
	src := `
enum BoolBox {
	case Wrap {
		value Bool
	}
	case Empty
}

enum PairBox {
	case Full {
		value (Int, Int)
	}
	case NoneX
}

def describeFlag(value BoolBox) Int =
	match value {
		case Wrap(true) => 1
		case Wrap(false) => 2
		case BoolBox.Empty => 3
	}

def describePair(value PairBox) Int =
	match value {
		case Full((left, right)) => left + right
		case PairBox.NoneX => 0
	}

def run() Int {
	return describeFlag(BoolBox.Wrap(false)) * 10 + describePair(PairBox.Full((4, 5)))
}
`

	in := New(parseProgram(t, src))
	value, err := in.Call("run")
	if err != nil {
		t.Fatalf("Call returned error: %v", err)
	}
	if value != int64(29) {
		t.Fatalf("expected 29, got %#v", value)
	}
}

func TestFunctionImplicitReturn(t *testing.T) {
	src := `
def suffix(value Str) Str {
	value + "!"
}

def run() Str {
	suffix("hello")
}
`

	in := New(parseProgram(t, src))
	value, err := in.Call("run")
	if err != nil {
		t.Fatalf("Call returned error: %v", err)
	}
	if value != "hello!" {
		t.Fatalf("expected hello!, got %#v", value)
	}
}

func TestUnitLambda(t *testing.T) {
	src := `
def run() Int {
	action () -> Unit = () -> OS.println("hi")
	action()
	return 0
}
`

	in := New(parseProgram(t, src))
	value, err := in.Call("run")
	if err != nil {
		t.Fatalf("Call returned error: %v", err)
	}
	if value != int64(0) {
		t.Fatalf("expected 0, got %#v", value)
	}
}

func TestExplicitUnitFunction(t *testing.T) {
	src := `
def log() Unit = "hello!"

def run() Int {
	log()
	return 0
}
`

	in := New(parseProgram(t, src))
	value, err := in.Call("run")
	if err != nil {
		t.Fatalf("Call returned error: %v", err)
	}
	if value != int64(0) {
		t.Fatalf("expected 0, got %#v", value)
	}
}

func TestUnitLiteralAndGroupedExpr(t *testing.T) {
	src := `
def run() Bool {
	unit Unit = ()
	value Int = (1)
	return value == 1
}
`

	in := New(parseProgram(t, src))
	value, err := in.Call("run")
	if err != nil {
		t.Fatalf("Call returned error: %v", err)
	}
	if value != true {
		t.Fatalf("expected true, got %#v", value)
	}
}

func TestZeroArgFunctionBindingSugar(t *testing.T) {
	src := `
def run() Int {
	action () -> Int = 1
	return action()
}
`

	in := New(parseProgram(t, src))
	value, err := in.Call("run")
	if err != nil {
		t.Fatalf("Call returned error: %v", err)
	}
	if value != int64(1) {
		t.Fatalf("expected 1, got %#v", value)
	}
}

func TestFunctionWithoutReturnTypeDoesNotImplicitlyReturn(t *testing.T) {
	src := `
def suffix(value Str) {
	value + "!"
}

def run() Bool {
	return suffix("hello") == "hello!"
}
`

	in := New(parseProgram(t, src))
	value, err := in.Call("run")
	if err != nil {
		t.Fatalf("Call returned error: %v", err)
	}
	if value != false {
		t.Fatalf("expected false, got %#v", value)
	}
}

func TestLocalFunctionStmt(t *testing.T) {
	src := `
def run() Int {
	boost Int = 2
	def add(value Int) Int = value + boost
	add(5)
}
`

	in := New(parseProgram(t, src))
	value, err := in.Call("run")
	if err != nil {
		t.Fatalf("Call returned error: %v", err)
	}
	if value != int64(7) {
		t.Fatalf("expected 7, got %#v", value)
	}
}

func TestVariadicFunctionAndMethod(t *testing.T) {
	src := `
class Printer {
}

impl Printer {
	def count(values Str...) Int = values.size()
}

def sum(values Int...) Int {
	var total Int = 0
	for value <- values {
		total += value
	}
	total
}

def run() Int {
	printer Printer = Printer()
	return sum(1, 2, 3) + printer.count("a", "b")
}
`

	in := New(parseProgram(t, src))
	value, err := in.Call("run")
	if err != nil {
		t.Fatalf("Call returned error: %v", err)
	}
	if value != int64(8) {
		t.Fatalf("expected 8, got %#v", value)
	}
}

func TestMultiAssignmentStmt(t *testing.T) {
	src := `
def run() Int {
	var a Int = 1
	var b Int = 2
	a, b := b, a + b
	a + b
}
`

	in := New(parseProgram(t, src))
	value, err := in.Call("run")
	if err != nil {
		t.Fatalf("Call returned error: %v", err)
	}
	if value != int64(5) {
		t.Fatalf("expected 5, got %#v", value)
	}
}

func TestTupleDestructuring(t *testing.T) {
	src := `
def run() Int {
	a (Int, Int) = (1, 2)
	b (Int, Int) = a
	c = a
	let (left Int, right Int) = c
	let (otherLeft Int, otherRight Int) = b
	left + right + otherLeft + otherRight
}
`

	in := New(parseProgram(t, src))
	value, err := in.Call("run")
	if err != nil {
		t.Fatalf("Call returned error: %v", err)
	}
	if value != int64(6) {
		t.Fatalf("expected 6, got %#v", value)
	}
}

func TestTupleMemberAccess(t *testing.T) {
	src := `
def run() Int {
	pair = (1, "x")
	return pair._1
}
`

	in := New(parseProgram(t, src))
	_, err := in.Call("run")
	if err == nil || err.Error() != "unknown member '_1' at 4:9" {
		t.Fatalf("expected unknown member error, got %#v", err)
	}
}

func TestTupleDestructuringAfterMethodReturn(t *testing.T) {
	src := `
class Counter {
}

impl Counter {
	def pair() (Int, Int) = (2, 3)
}

def run() Int {
	counter = Counter()
	let (left Int, right Int) = counter.pair()
	return left + right
}
`

	in := New(parseProgram(t, src))
	value, err := in.Call("run")
	if err != nil {
		t.Fatalf("Call returned error: %v", err)
	}
	if value != int64(5) {
		t.Fatalf("expected 5, got %#v", value)
	}
}

func TestMethodReferenceRequiresCall(t *testing.T) {
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
	bad = counter.inc
	return 0
}
`

	in := New(parseProgram(t, src))
	_, err := in.Call("run")
	if err == nil {
		t.Fatalf("expected runtime error")
	}
	if !strings.Contains(err.Error(), "must be called with ()") {
		t.Fatalf("unexpected runtime error: %v", err)
	}
}

func TestLambdaFunctionValue(t *testing.T) {
	src := `
def run() Int {
	add = (x Int) -> x + 1
	return add(2)
}
`

	in := New(parseProgram(t, src))
	value, err := in.Call("run")
	if err != nil {
		t.Fatalf("Call returned error: %v", err)
	}
	if value != int64(3) {
		t.Fatalf("expected 3, got %#v", value)
	}
}

func TestBlockLambdaFunctionValue(t *testing.T) {
	src := `
def run() Int {
	add = (x Int) -> {
		y Int = x + 1
		return y
	}
	return add(2)
}
`

	in := New(parseProgram(t, src))
	value, err := in.Call("run")
	if err != nil {
		t.Fatalf("Call returned error: %v", err)
	}
	if value != int64(3) {
		t.Fatalf("expected 3, got %#v", value)
	}
}

func TestClassEqualityUsesEquals(t *testing.T) {
	src := `
class Counter {
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

	in := New(parseProgram(t, src))
	value, err := in.Call("run")
	if err != nil {
		t.Fatalf("Call returned error: %v", err)
	}
	if value != true {
		t.Fatalf("expected true, got %#v", value)
	}
}

func TestBuiltinCollectionsAndTerm(t *testing.T) {
	src := `
object Ascending with Ordering[Int] {
	def compare(left Int, right Int) Int = left - right
}
def run() Int {
	items = List(1, 2)
	items.append(3)
	items.sort(Ascending)

	values = Map("a" : 1)
	values.put("b", 2)

	seen = Set(1, 2)
	if seen.contains(2) {
		OS.println("ok", "done")
	}

	return items.get(0).getOr(0) + values.get("a").getOr(0) + values.size() + seen.size()
}
`

	in := New(parseProgram(t, src))

	oldStdout := os.Stdout
	reader, writer, err := os.Pipe()
	if err != nil {
		t.Fatalf("Pipe returned error: %v", err)
	}
	os.Stdout = writer

	value, callErr := in.Call("run")

	_ = writer.Close()
	os.Stdout = oldStdout
	output, _ := io.ReadAll(reader)
	_ = reader.Close()

	if callErr != nil {
		t.Fatalf("Call returned error: %v", callErr)
	}
	if value != int64(6) {
		t.Fatalf("expected 6, got %#v", value)
	}
	if strings.TrimSpace(string(output)) != "ok done" {
		t.Fatalf("expected OS output 'ok', got %q", string(output))
	}
}

func TestArrayElementConstruction(t *testing.T) {
	src := `
class Box {
	value Int
}

def run() Int {
	values Array[Int] = Array(4, 5, 6)
	boxes Array[Box] = Array(Box(7), Box(8))
	return values[0] * 1000 + values[1] * 100 + values[2] * 10 + boxes[1].value
}
`

	in := New(parseProgram(t, src))
	value, err := in.Call("run")
	if err != nil {
		t.Fatalf("Call returned error: %v", err)
	}
	if value != int64(4568) {
		t.Fatalf("expected 4568, got %#v", value)
	}
}

func TestListSortWithOrdering(t *testing.T) {
	src := `
object Descending with Ordering[Int] {
	def compare(left Int, right Int) Int = right - left
}

def run() Int {
	items = List(3, 1, 4, 2)
	items.sort(Descending)
	return items.get(0).getOr(0) * 1000 + items.get(1).getOr(0) * 100 + items.get(2).getOr(0) * 10 + items.get(3).getOr(0)
}
`

	in := New(parseProgram(t, src))
	value, err := in.Call("run")
	if err != nil {
		t.Fatalf("Call returned error: %v", err)
	}
	if value != int64(4321) {
		t.Fatalf("expected 4321, got %#v", value)
	}
}

func TestCustomAndCollectionOperators(t *testing.T) {
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

	items = List(1, 2)
	items2 = items :+ 3
	merged = items2 ++ List(4, 5)

	seen = Set(1, 2)
	all = seen ++ Set(3)

	return neg[0] + diff[0] + trimmed[1] + merged[4] + all.size()
}
`

	in := New(parseProgram(t, src))
	value, err := in.Call("run")
	if err != nil {
		t.Fatalf("Call returned error: %v", err)
	}
	if value != int64(10) {
		t.Fatalf("expected 10, got %#v", value)
	}
}

func TestListMapFlatMapForEach(t *testing.T) {
	src := `
def run() Int {
	items = List(1, 2, 3)
	doubled = items.map((item Int) -> item * 2)
	doubled2 = items.map(item -> item * 2)
	expanded = items.flatMap((item Int) -> List(item, item + 10))
	doubled.forEach((item Int) -> OS.println("item " + item))
	return doubled.get(2).getOr(0) * 100 + doubled2.get(1).getOr(0) * 10 + expanded.get(5).getOr(0)
}
`

	in := New(parseProgram(t, src))

	oldStdout := os.Stdout
	reader, writer, err := os.Pipe()
	if err != nil {
		t.Fatalf("Pipe returned error: %v", err)
	}
	os.Stdout = writer

	value, callErr := in.Call("run")

	_ = writer.Close()
	os.Stdout = oldStdout
	output, _ := io.ReadAll(reader)
	_ = reader.Close()

	if callErr != nil {
		t.Fatalf("Call returned error: %v", callErr)
	}
	if value != int64(653) {
		t.Fatalf("expected 653, got %#v", value)
	}
	if strings.TrimSpace(string(output)) != "item 2\nitem 4\nitem 6" {
		t.Fatalf("unexpected output %q", string(output))
	}
}

func TestSetAndMapHigherOrderMethods(t *testing.T) {
	src := `
def run() Int {
	seen = Set(1, 2, 3)
	doubled = seen.map((item Int) -> item * 2)
	expanded = seen.flatMap((item Int) -> Set(item, item + 10))
	filtered = seen.filter((item Int) -> item > 1)
	setTotal = seen.fold(0, (acc Int, item Int) -> acc + item)
	setReduced = seen.reduce((left Int, right Int) -> left + right)
	setHasBig = seen.exists((item Int) -> item > 2)
	setAllPositive = seen.forAll((item Int) -> item > 0)
	seen.forEach((item Int) -> OS.println("set " + item))

	values = Map("a" : 1, "b" : 2)
	mapped = values.map((key Str, value Int) -> value * 10)
	mappedValues = values.mapValues((value Int) -> value * 100)
	expandedValues = values.flatMap((key Str, value Int) -> List(value, value + 10))
	filteredMap = values.filter((key Str, value Int) -> value > 1)
	mapTotal = values.fold(0, (acc Int, key Str, value Int) -> acc + value)
	mapReduced = values.reduce((leftKey Str, leftValue Int, rightKey Str, rightValue Int) -> (rightKey, rightValue))
	mapHasB = values.exists((key Str, value Int) -> key == "b")
	mapAllSmall = values.forAll((key Str, value Int) -> value < 3)
	values.forEach((key Str, value Int) -> OS.println("pair " + key + " " + value))

	var total = 0
	for item Int <- seen {
		total += item
	}
	for key Str, value Int <- values {
		total += value
	}

	unwrap reducedPair <- mapReduced else {
		0
	}
	let (reducedKey, reducedValue) = reducedPair
	if expanded.contains(12) && setHasBig && setAllPositive && mapHasB && mapAllSmall {
		if reducedKey == "b" {
			return total * 1000000 + mapped.get(0).getOr(0) * 100000 + mappedValues["b"].getOr(0) * 10000 + expandedValues.get(3).getOr(0) * 1000 + doubled.size() * 100 + filtered.size() * 10 + setTotal + setReduced.getOr(0) + filteredMap.size() + mapTotal + reducedValue
		}
	}
	return 0
}
	`

	in := New(parseProgram(t, src))

	oldStdout := os.Stdout
	reader, writer, err := os.Pipe()
	if err != nil {
		t.Fatalf("Pipe returned error: %v", err)
	}
	os.Stdout = writer

	value, callErr := in.Call("run")

	_ = writer.Close()
	os.Stdout = oldStdout
	output, _ := io.ReadAll(reader)
	_ = reader.Close()

	if callErr != nil {
		t.Fatalf("Call returned error: %v", callErr)
	}
	if value != int64(12012338) {
		t.Fatalf("expected 12012338, got %#v", value)
	}
	if strings.TrimSpace(string(output)) != "set 1\nset 2\nset 3\npair a 1\npair b 2" {
		t.Fatalf("unexpected output %q", string(output))
	}
}

func TestOptionRuntime(t *testing.T) {
	src := `
def run() Int {
	found = Some(5)
	missing = None()
	unwrap value <- found else return 0
	return value + missing.getOr(2)
}
`

	in := New(parseProgram(t, src))
	value, err := in.Call("run")
	if err != nil {
		t.Fatalf("Call returned error: %v", err)
	}
	if value != int64(7) {
		t.Fatalf("expected 7, got %#v", value)
	}
}

func TestBuiltinRuntimeUsesPredefMethodSurface(t *testing.T) {
	src := `
def run() Int {
	items = List(10, 20, 30)
	head = items.head().getOr(0)
	rest = items.tail()
	wasEmpty = items.isEmpty()
	removed = items.remove(1)
	last = items.removeLast()
	emptyItems = []
	nowEmpty = emptyItems.isEmpty()
	empty = None()

	OS.print("left", "-", "right")

	if empty.isEmpty() && !wasEmpty && nowEmpty {
		return head * 100000 + rest.size() * 10000 + removed.getOr(0) * 100 + last.getOr(0)
	}
	return 0
}
`

	in := New(parseProgram(t, src))

	oldStdout := os.Stdout
	reader, writer, err := os.Pipe()
	if err != nil {
		t.Fatalf("Pipe returned error: %v", err)
	}
	os.Stdout = writer

	value, callErr := in.Call("run")

	_ = writer.Close()
	os.Stdout = oldStdout
	output, _ := io.ReadAll(reader)
	_ = reader.Close()

	if callErr != nil {
		t.Fatalf("Call returned error: %v", callErr)
	}
	if value != int64(1022030) {
		t.Fatalf("expected 1022030, got %#v", value)
	}
	if string(output) != "left-right" {
		t.Fatalf("unexpected output %q", string(output))
	}
}

func TestStringConcatenation(t *testing.T) {
	src := `
def run() Str {
	count Int = 10
	return "counter " + count
}
`

	in := New(parseProgram(t, src))
	value, err := in.Call("run")
	if err != nil {
		t.Fatalf("Call returned error: %v", err)
	}
	if value != "counter 10" {
		t.Fatalf("expected %q, got %#v", "counter 10", value)
	}
}

func TestStringInterpolation(t *testing.T) {
	src := `
def run() Str {
	name Str = "world"
	count Int = 2
	return "hello $name ${count + 1} \$done"
}
`

	in := New(parseProgram(t, src))
	value, err := in.Call("run")
	if err != nil {
		t.Fatalf("Call returned error: %v", err)
	}
	if value != "hello world 3 $done" {
		t.Fatalf("expected %q, got %#v", "hello world 3 $done", value)
	}
}

func TestMultilineString(t *testing.T) {
	src := `
def run() Str {
	return """
hello
$name
\n
"""
}
`

	in := New(parseProgram(t, src))
	value, err := in.Call("run")
	if err != nil {
		t.Fatalf("Call returned error: %v", err)
	}
	if value != "\nhello\n$name\n\n\n" {
		t.Fatalf("expected %q, got %#v", "\nhello\n$name\n\n\n", value)
	}
}

func TestIfAndForYieldExpressions(t *testing.T) {
	src := `
def run() Int {
	values = [1, 2, 3]
	label Int = if true {
		10
	} else {
		20
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
	OS.println("items " + items.size() + " " + items2.size())
	return label + items.size() + items2.size()
}
`

	in := New(parseProgram(t, src))

	oldStdout := os.Stdout
	reader, writer, err := os.Pipe()
	if err != nil {
		t.Fatalf("Pipe returned error: %v", err)
	}
	os.Stdout = writer

	value, callErr := in.Call("run")

	_ = writer.Close()
	os.Stdout = oldStdout
	output, _ := io.ReadAll(reader)
	_ = reader.Close()

	if callErr != nil {
		t.Fatalf("Call returned error: %v", callErr)
	}
	if value != int64(22) {
		t.Fatalf("expected 22, got %#v", value)
	}
	if strings.TrimSpace(string(output)) != "items 9 3" {
		t.Fatalf("expected output %q, got %q", "items 9 3", string(output))
	}
}

func TestArrayConstructorAndSize(t *testing.T) {
	src := `
def run() Int {
	values = Array.ofLength(5)
	values[1] := 7
	return values.size()
}
`

	in := New(parseProgram(t, src))
	value, err := in.Call("run")
	if err != nil {
		t.Fatalf("Call returned error: %v", err)
	}
	if value != int64(5) {
		t.Fatalf("expected 5, got %#v", value)
	}
}

func TestIsExpression(t *testing.T) {
	src := `
class Counter {
}

def run() Bool {
	counter = Counter()
	return counter is Counter && "x" is Str && !(counter is Str)
}
`

	in := New(parseProgram(t, src))
	value, err := in.Call("run")
	if err != nil {
		t.Fatalf("Call returned error: %v", err)
	}
	if value != true {
		t.Fatalf("expected true, got %#v", value)
	}
}

func TestInterfaceInheritanceIsExpression(t *testing.T) {
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

def run() Bool {
	rabbit = Rabbit()
	return rabbit is Acrobat && rabbit is Hopper && rabbit is Jumper
}
`

	in := New(parseProgram(t, src))
	value, err := in.Call("run")
	if err != nil {
		t.Fatalf("Call returned error: %v", err)
	}
	if value != true {
		t.Fatalf("expected true, got %#v", value)
	}
}

func TestInterfaceDefaultMethod(t *testing.T) {
	src := `
interface Hopper {
	def hop() Str = "hop"
}

interface Jumper {
	def jump(steps Int) Str
}

class Rabbit with Hopper, Jumper {
}

impl Rabbit {
	def jump(steps Int) Str = "jump " + steps
}

def run() Str {
	rabbit Hopper = Rabbit()
	jumper Jumper = Rabbit()
	return rabbit.hop() + " / " + jumper.jump(2)
}
`

	in := New(parseProgram(t, src))
	value, err := in.Call("run")
	if err != nil {
		t.Fatalf("Call returned error: %v", err)
	}
	if value != "hop / jump 2" {
		t.Fatalf("expected %q, got %#v", "hop / jump 2", value)
	}
}

func TestRecordUpdateExpr(t *testing.T) {
	src := `
record Amount {
	amount Int
	description Str
}

def run() Bool {
	value = Amount(10, "x")
	updated = value with { amount = 42 }
	return value.amount == 10 && updated.amount == 42 && updated.description == "x"
}
`

	in := New(parseProgram(t, src))
	value, err := in.Call("run")
	if err != nil {
		t.Fatalf("Call returned error: %v", err)
	}
	if value != true {
		t.Fatalf("expected true, got %#v", value)
	}
}

func TestAnonymousInterfaceExpr(t *testing.T) {
	src := `
interface Reader {
	def read() Str
}

interface Closer {
	def close() Unit
}

def run() Bool {
	handler = Reader with Closer {
		def read() Str = "x"
		def close() Unit = ()
	}
	handler.close()
	return handler.read() == "x"
}
`

	in := New(parseProgram(t, src))
	value, err := in.Call("run")
	if err != nil {
		t.Fatalf("Call returned error: %v", err)
	}
	if value != true {
		t.Fatalf("expected true, got %#v", value)
	}
}

func TestAnonymousRecordExpr(t *testing.T) {
	src := `
def describe(user { name Str, age Int }) Int {
	return user.age
}

def makeCounter(base Int) { count Int, next Int } =
	record {
		count = base
		next = base + 1
	}

def run() Bool {
	full = record {
		name = "Ana"
		age = 10
		city = "NYC"
	}
	name = "Cara"
	age = 14
	narrow { name Str, age Int } = full
	positional { name Str, age Int } = record { "Ben", 12 }
	contextual { name Str, age Int } = record { name, age }
	counter { count Int, next Int } = makeCounter(5)
	mixed = record { a = 5, c = 7,
		b = 8
	}
	return describe(full) == 10 &&
		describe(record { "Cara", 14 }) == 14 &&
		narrow.name == "Ana" &&
		positional.name == "Ben" &&
		positional.age == 12 &&
		contextual.name == "Cara" &&
		contextual.age == 14 &&
		counter.next == 6 &&
		counter.count == 5 &&
		full.city == "NYC" &&
		mixed.a + mixed.b + mixed.c == 20
}
`

	in := New(parseProgram(t, src))
	value, err := in.Call("run")
	if err != nil {
		t.Fatalf("Call returned error: %v", err)
	}
	if value != true {
		t.Fatalf("expected true, got %#v", value)
	}
}

func TestAnonymousRecordToClassAndRecord(t *testing.T) {
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

def makeTeam(owner Person) Team {
	return Team(record {
		name = "Core"
		owner = owner
	})
}

def run() Bool {
	userRecord = record {
		name = "Ana"
		age = 10
	}
	user User = User(userRecord)
	person Person = Person(record { "Ben", 12 })
	team = makeTeam(Person(record {
		name = "Cy"
		age = 7
	}))
	return user.name == "Ana" &&
		user.age == 10 &&
		person.name == "Ben" &&
		person.age == 12 &&
		person.city == "NYC" &&
		team.owner.name == "Cy" &&
		team.owner.age == 7 &&
		team.owner.city == "NYC"
}
`

	in := New(parseProgram(t, src))
	value, err := in.Call("run")
	if err != nil {
		t.Fatalf("Call returned error: %v", err)
	}
	if value != true {
		t.Fatalf("expected true, got %#v", value)
	}
}

func TestRecordAndClassDestructuring(t *testing.T) {
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

	in := New(parseProgram(t, src))
	value, err := in.Call("run")
	if err != nil {
		t.Fatalf("Call returned error: %v", err)
	}
	if value != int64(12) {
		t.Fatalf("expected 12, got %#v", value)
	}
}

func TestDestructuringSkipBinding(t *testing.T) {
	src := `
record Triple {
	first Int
	middle Str
	last Str
}

def run() Str {
	let (a Int, _, c Str) = Triple(1, "drop", "keep")
	return c
}
`

	in := New(parseProgram(t, src))
	value, err := in.Call("run")
	if err != nil {
		t.Fatalf("Call returned error: %v", err)
	}
	if value != "keep" {
		t.Fatalf("expected keep, got %#v", value)
	}
}

func TestNamedCallArguments(t *testing.T) {
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
	left Int = doSomething(b = 5, a = "go")
	right Int = counter.set(label = "ok", value = 7)
	return left + right
}
`

	in := New(parseProgram(t, src))
	value, err := in.Call("run")
	if err != nil {
		t.Fatalf("Call returned error: %v", err)
	}
	if value != int64(12) {
		t.Fatalf("expected 12, got %#v", value)
	}
}

func TestObjectSingletonAccess(t *testing.T) {
	src := `
object A {
	var count Int = 2

	def value() Int = count

	def test(a Int) Int {
		return a + this.value()
	}
}

def run() Int {
	return A.test(5)
}
`

	in := New(parseProgram(t, src))
	value, err := in.Call("run")
	if err != nil {
		t.Fatalf("Call returned error: %v", err)
	}
	if value != int64(7) {
		t.Fatalf("expected 7, got %#v", value)
	}
}

func TestObjectApplyCall(t *testing.T) {
	src := `
object Range {
	def apply(end Int) Int = end
}

def run() Int {
	return Range.apply(5)
}
`

	in := New(parseProgram(t, src))
	value, err := in.Call("run")
	if err != nil {
		t.Fatalf("Call returned error: %v", err)
	}
	if value != int64(5) {
		t.Fatalf("expected 5, got %#v", value)
	}
}

func TestObjectDirectCall(t *testing.T) {
	src := `
object Range {
	def apply(end Int) Int = end
}

def run() Int {
	return Range(5)
}
`

	in := New(parseProgram(t, src))
	_, err := in.Call("run")
	if err == nil {
		t.Fatal("expected direct object call to fail")
	}
	if !strings.Contains(err.Error(), "object 'Range' is not callable") {
		t.Fatalf("expected not callable error, got %v", err)
	}
}

func TestClassApplyCall(t *testing.T) {
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

	in := New(parseProgram(t, src))
	value, err := in.Call("run")
	if err != nil {
		t.Fatalf("Call returned error: %v", err)
	}
	if value != int64(12) {
		t.Fatalf("expected 12, got %#v", value)
	}
}

func TestRecordApplyCall(t *testing.T) {
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

	in := New(parseProgram(t, src))
	value, err := in.Call("run")
	if err != nil {
		t.Fatalf("Call returned error: %v", err)
	}
	if value != int64(12) {
		t.Fatalf("expected 12, got %#v", value)
	}
}

func TestImplicitPrimaryConstructorAndThisDelegation(t *testing.T) {
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

	def value() Int = count
}

def run() Int {
	left Counter = Counter(1, "x")
	right Counter = Counter(seed = 3)
	return left.value() + right.value()
}
`

	in := New(parseProgram(t, src))
	value, err := in.Call("run")
	if err != nil {
		t.Fatalf("Call returned error: %v", err)
	}
	if value != int64(4) {
		t.Fatalf("expected 4, got %#v", value)
	}
}

func TestYieldLoopsWithImmutableBindings(t *testing.T) {
	src := `
def run() Int {
	items = for {
		item <- [1, 2, 3]
		next = item + 1
	} yield {
		next
	}

	unwrap first <- items.get(0) else return 0
	unwrap second <- items.get(1) else return 0
	unwrap third <- items.get(2) else return 0
	return first + second + third
}
`

	in := New(parseProgram(t, src))
	value, err := in.Call("run")
	if err != nil {
		t.Fatalf("Call returned error: %v", err)
	}
	if value != int64(9) {
		t.Fatalf("expected 9, got %#v", value)
	}
}

func TestMapIndexAssignment(t *testing.T) {
	src := `
def run() Int {
	values = Map("a" : 1)
	values["b"] := 2
	return values.get("b").getOr(0)
}
`

	in := New(parseProgram(t, src))
	value, err := in.Call("run")
	if err != nil {
		t.Fatalf("Call returned error: %v", err)
	}
	if value != int64(2) {
		t.Fatalf("expected 2, got %#v", value)
	}
}

func TestWhileLoop(t *testing.T) {
	src := `
def run() Int {
	var total Int = 0
	while total < 3 {
		total += 1
	}
	return total
}
`

	in := New(parseProgram(t, src))
	value, err := in.Call("run")
	if err != nil {
		t.Fatalf("Call returned error: %v", err)
	}
	if value != int64(3) {
		t.Fatalf("expected 3, got %#v", value)
	}
}

func TestYieldLoopsWithMutableBindings(t *testing.T) {
	src := `
def run() Int {
	items = for {
		item <- [1, 2, 3]
		var total = item
	} yield {
		total += 1
		total
	}

	unwrap first <- items.get(0) else return 0
	unwrap second <- items.get(1) else return 0
	unwrap third <- items.get(2) else return 0
	return first + second + third
}
`

	in := New(parseProgram(t, src))
	value, err := in.Call("run")
	if err != nil {
		t.Fatalf("Call returned error: %v", err)
	}
	if value != int64(9) {
		t.Fatalf("expected 9, got %#v", value)
	}
}

func TestIfOptionBinding(t *testing.T) {
	src := `
def run() Int {
	found Option[Int] = Some(5)
	missing Option[Int] = None()
	var total Int = 0
	if let Some(item) = found {
		total := total + item
	}
	if let Some(item) = missing {
		total := total + item
	}
	return total
}
`

	in := New(parseProgram(t, src))
	value, err := in.Call("run")
	if err != nil {
		t.Fatalf("Call returned error: %v", err)
	}
	if value != int64(5) {
		t.Fatalf("expected 5, got %#v", value)
	}
}

func TestIfOptionDestructuring(t *testing.T) {
	src := `
def run() Str {
	found Option[(Int, Str, Bool)] = Some((1, "ok", true))
	if let Some(item) = found {
		let (_, name, _) = item
		return name
	}
	return "missing"
}
`

	in := New(parseProgram(t, src))
	value, err := in.Call("run")
	if err != nil {
		t.Fatalf("Call returned error: %v", err)
	}
	if value != "ok" {
		t.Fatalf("expected %q, got %#v", "ok", value)
	}
}

func TestUnwrapStmtOptionResultEither(t *testing.T) {
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

def run() Str {
	optionValue = plusOneOption(Some(4)).getOr(0)
	resultValue = plusOneResult(Ok(5)).getOr(0)
	eitherValue = plusOneEither(Right(6)).getOr(0)
	comboValue = twoEithers(Right(7), Right("abc")).getOr(0)

	if plusOneOption(None()).isEmpty() &&
	   plusOneResult(Err("bad")).isErr() &&
	   plusOneEither(Left("nope")).isLeft() &&
	   twoEithers(Right(7), Left("size bad")).isLeft() {
		return "${optionValue}-${resultValue}-${eitherValue}-${comboValue}"
	}
	return "broken"
}
`

	in := New(parseProgram(t, src))
	value, err := in.Call("run")
	if err != nil {
		t.Fatalf("Call returned error: %v", err)
	}
	if value != "5-6-7-10" {
		t.Fatalf("expected %q, got %#v", "5-6-7-10", value)
	}
}

func TestGuardStmt(t *testing.T) {
	src := `
def runSome(value Option[Int]) Result[Int, Str] {
	unwrap item <- value else {
		Err("missing")
	}
	return Ok(item + 1)
}

def runNone(value Option[Int]) Result[Int, Str] {
	unwrap item <- value else {
		Err("missing")
	}
	return Ok(item + 1)
}

def run() Str {
	someValue = runSome(Some(4))
	noneValue = runNone(None())
	if someValue.getOr(0) == 5 && noneValue.getError() == "missing" {
		return "ok"
	}
	return "broken"
}
`

	in := New(parseProgram(t, src))
	value, err := in.Call("run")
	if err != nil {
		t.Fatalf("Call returned error: %v", err)
	}
	if value != "ok" {
		t.Fatalf("expected %q, got %#v", "ok", value)
	}
}

func TestGuardBlockStmt(t *testing.T) {
	src := `
def runSome(left Option[Int], right Option[Int]) Result[Int, Str] {
	unwrap {
		a <- left
		b <- right
	} else {
		Err("missing")
	}
	return Ok(a + b)
}

def runNone(left Option[Int], right Option[Int]) Result[Int, Str] {
	unwrap {
		a <- left
		b <- right
	} else {
		Err("missing")
	}
	return Ok(a + b)
}

def run() Str {
	someValue = runSome(Some(4), Some(5))
	noneValue = runNone(Some(4), None())
	if someValue.getOr(0) == 9 && noneValue.getError() == "missing" {
		return "ok"
	}
	return "broken"
}
`

	in := New(parseProgram(t, src))
	value, err := in.Call("run")
	if err != nil {
		t.Fatalf("Call returned error: %v", err)
	}
	if value != "ok" {
		t.Fatalf("expected %q, got %#v", "ok", value)
	}
}

func TestUnwrapBlockStmt(t *testing.T) {
	src := `
def runSome(left Option[Int], right Option[Int]) Option[Int] {
	unwrap {
		a <- left
		b <- right
	}
	return Some(a + b)
}

def runNone(left Option[Int], right Option[Int]) Option[Int] {
	unwrap {
		a <- left
		b <- right
	}
	return Some(a + b)
}

def run() Str {
	someValue = runSome(Some(4), Some(5))
	noneValue = runNone(Some(4), None())
	return "${someValue.getOr(0)}-${noneValue.isEmpty()}"
}
`

	in := New(parseProgram(t, src))
	out, err := in.Call("run")
	if err != nil {
		t.Fatalf("Call returned error: %v", err)
	}
	if out != "9-true" {
		t.Fatalf("expected output %q, got %#v", "9-true", out)
	}
}
func TestEnumMethodsUseTopLevelImpl(t *testing.T) {
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
	return left.describe() + "-" + right.describe() + "-" + left.tag + "-" + right.tag
}
`

	in := New(parseProgram(t, src))
	value, err := in.Call("run")
	if err != nil {
		t.Fatalf("Call returned error: %v", err)
	}
	if value != "bad-num 7-left-right" {
		t.Fatalf("expected %q, got %#v", "bad-num 7-left-right", value)
	}
}

func TestMapIndexReturnsOption(t *testing.T) {
	src := `
def run() Str {
	entries = Map("a": 1, "b": 2)
	present = entries["a"]
	missing = entries["z"]
	if present.getOr(0) == 1 && missing.isEmpty() {
		return "ok"
	}
	return "broken"
}
`

	in := New(parseProgram(t, src))
	value, err := in.Call("run")
	if err != nil {
		t.Fatalf("Call returned error: %v", err)
	}
	if value != "ok" {
		t.Fatalf("expected %q, got %#v", "ok", value)
	}
}

func TestPartialMatchAndPlaceholderMatchIf(t *testing.T) {
	src := `
enum MaybeInt {
	case NoneX
	case SomeX {
		value Int
	}
}

def run() Str {
	values = List(1, 6, 3)
	ifMapped = values.map(if _ > 5 then 10 else 8)
	options = List(MaybeInt.SomeX(1), MaybeInt.NoneX, MaybeInt.SomeX(3))
	matchMapped = options.map(match {
	case SomeX(x) => x + 1
	case NoneX => 0
	})
	partialMapped = options.map(partial {
	case SomeX(x) => x + 1
	})
	unwrap firstPartial <- partialMapped.get(0) else {
		""
	}
	unwrap secondPartial <- partialMapped.get(1) else {
		""
	}
	return "${ifMapped.get(0).getOr(0)}-${ifMapped.get(1).getOr(0)}-${matchMapped.get(2).getOr(0)}-${firstPartial.getOr(0)}-${secondPartial.isEmpty()}"
}
`

	in := New(parseProgram(t, src))
	value, err := in.Call("run")
	if err != nil {
		t.Fatalf("Call returned error: %v", err)
	}
	if value != "8-10-4-2-true" {
		t.Fatalf("expected %q, got %#v", "8-10-4-2-true", value)
	}
}

func TestPartialMethod(t *testing.T) {
	src := `
class Classifier {
}

impl Classifier {
	partial classify(value Int) Int {
	case 3 => 5
	case 4 => 0
	}
}

def run() Str {
	classifier = Classifier()
	first = classifier.classify(3)
	second = classifier.classify(8)
	return "${first.getOr(0)}-${second.isEmpty()}"
}
`

	in := New(parseProgram(t, src))
	value, err := in.Call("run")
	if err != nil {
		t.Fatalf("Call returned error: %v", err)
	}
	if value != "5-true" {
		t.Fatalf("expected %q, got %#v", "5-true", value)
	}
}

func TestTrailingBlockLambda(t *testing.T) {
	src := `
def run() Int {
	values = [1, 2, 3]
	mapped = values.map { x -> x + 1 }
	return mapped.get(2).getOr(0)
}
`

	in := New(parseProgram(t, src))
	value, err := in.Call("run")
	if err != nil {
		t.Fatalf("Call returned error: %v", err)
	}
	if value != int64(4) {
		t.Fatalf("expected 4, got %#v", value)
	}
}

func TestStrSize(t *testing.T) {
	src := `
def run() Int {
	return "hello".size() + "мир".size()
}
`

	in := New(parseProgram(t, src))
	value, err := in.Call("run")
	if err != nil {
		t.Fatalf("Call returned error: %v", err)
	}
	if value != int64(8) {
		t.Fatalf("expected 8, got %#v", value)
	}
}

func TestStrSplitAndIndexOf(t *testing.T) {
	src := `
def run() Int {
	parts = "cat dog cat".split(" ")
	return parts.size() * 100 + "cat dog cat".indexOf("dog") * 10 + "cat dog cat".indexOf("bird")
}
`

	in := New(parseProgram(t, src))
	value, err := in.Call("run")
	if err != nil {
		t.Fatalf("Call returned error: %v", err)
	}
	if value != int64(339) {
		t.Fatalf("expected 339, got %#v", value)
	}
}

func TestNestedBlockExpressions(t *testing.T) {
	src := `
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
`

	in := New(parseProgram(t, src))
	value, err := in.Call("run")
	if err != nil {
		t.Fatalf("Call returned error: %v", err)
	}
	if value != int64(14) {
		t.Fatalf("expected 14, got %#v", value)
	}
}

func TestNestedBlockValueStatements(t *testing.T) {
	src := `
def run() Int {
	fromIf = {
		if false {
			10
		} else {
			20
		}
	}
	fromYield = {
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

	in := New(parseProgram(t, src))
	value, err := in.Call("run")
	if err != nil {
		t.Fatalf("Call returned error: %v", err)
	}
	if value != int64(40) {
		t.Fatalf("expected 40, got %#v", value)
	}
}

func TestPlaceholderLambdaShorthand(t *testing.T) {
	src := `
def applyTwice(f (Int) -> Int, value Int) Int = f(f(value))

def run() Int {
	inc (Int) -> Int = _ + 1
	items = List(1, 2, 3)
	mapped = items.map(_ + 1)
	opt = Some(5)
	mappedOpt = opt.map(_ -> 1)
	return applyTwice(inc, mapped.get(0).getOr(0)) + mappedOpt.getOr(0)
}
`

	in := New(parseProgram(t, src))
	value, err := in.Call("run")
	if err != nil {
		t.Fatalf("Call returned error: %v", err)
	}
	if value != int64(5) {
		t.Fatalf("expected 5, got %#v", value)
	}
}

func TestTupleDestructuringLambdas(t *testing.T) {
	src := `
def run() Int {
	pairs = List(("a", 1), ("bb", 2))
	pairMapped = pairs.map((key, value) -> key.size() + value)
	pairKeys = pairs.map((key, _) -> key)
	pairIgnored = pairs.map((_, value) -> value * 2)
	tuple4s = List((1, 2, 3, 4), (4, 5, 6, 7))
	tuple4Mapped = tuple4s.map((first, _, third, _) -> first + third)
	entries = Map("a": 1, "bbb": 2)
	mapMapped = entries.map((key, value) -> key.size() + value)
	return pairMapped.get(0).getOr(0) +
		pairMapped.get(1).getOr(0) +
		pairKeys.get(1).getOr("").size() +
		mapMapped.get(1).getOr(0) +
		pairIgnored.get(1).getOr(0) +
		tuple4Mapped.get(1).getOr(0)
}
`

	in := New(parseProgram(t, src))
	value, err := in.Call("run")
	if err != nil {
		t.Fatalf("Call returned error: %v", err)
	}
	if value != int64(27) {
		t.Fatalf("expected 27, got %#v", value)
	}
}
