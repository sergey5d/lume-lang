package java

import (
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"testing"

	"a-lang/lower"
	"a-lang/parser"
	"a-lang/predef"
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

func lowerProgram(t *testing.T, src string) *lower.Program {
	t.Helper()
	program := parseProgram(t, src)
	typesResult := typecheck.Analyze(program)
	if len(typesResult.Diagnostics) != 0 {
		t.Fatalf("expected no type diagnostics, got %#v", typesResult.Diagnostics)
	}
	typedProgram, err := typed.Build(program, typesResult)
	if err != nil {
		t.Fatalf("typed.Build returned error: %v", err)
	}
	lowered, err := lower.ProgramFromTyped(typedProgram)
	if err != nil {
		t.Fatalf("ProgramFromTyped returned error: %v", err)
	}
	return lowered
}

func TestGenerateCompilesWithJavac(t *testing.T) {
	src := `
class Counter {
	hidden var count Int
}

impl Counter {
	def init(count Int) {
		this.count = count
	}

	def bump(delta Int) Int {
		this.count += delta
		return this.count
	}
}

object MathBox {
	hidden base Int = 2

	def add(x Int) Int = x + this.base
}

seed Int = 3

def accumulate(values Array[Int]) Int {
	var total Int = 0
	for value <- values {
		total += value
	}
	return total
}

def run() Int {
	counter Counter = Counter(seed)
	values Array[Int] = Array(1, 2, 3)
	values[0] := values[0] + 1
	if seed > 0 {
		return MathBox.add(counter.bump(accumulate(values)))
	}
	return 0
}
`

	lowered := lowerProgram(t, src)
	source, err := GenerateForPackage(lowered, "")
	if err != nil {
		t.Fatalf("Generate returned error: %v", err)
	}

	text := string(source)
	if !strings.Contains(text, "import alang.stdlib.*;") {
		t.Fatalf("expected stdlib import in generated Java, got:\n%s", text)
	}
	if !strings.Contains(text, "public final class Pkg_Default") {
		t.Fatalf("expected package holder class in generated Java, got:\n%s", text)
	}
	if !strings.Contains(text, "public static void main(String[] args)") {
		t.Fatalf("expected java main bridge in generated Java, got:\n%s", text)
	}
	if !strings.Contains(text, "final class Counter") || !strings.Contains(text, "final class Obj_MathBox") {
		t.Fatalf("expected object backing class in generated Java, got:\n%s", text)
	}
	if !strings.Contains(text, "new long[] {1L, 2L, 3L}") {
		t.Fatalf("expected Array(...) lowering in generated Java, got:\n%s", text)
	}

	tmpDir := t.TempDir()
	if err := WriteStdlibSupport(tmpDir); err != nil {
		t.Fatalf("WriteStdlibSupport returned error: %v", err)
	}
	path := filepath.Join(tmpDir, "Pkg_Default.java")
	if err := os.WriteFile(path, source, 0o644); err != nil {
		t.Fatalf("write generated source: %v", err)
	}

	javaFiles := collectJavaFiles(t, tmpDir)
	cmd := exec.Command("javac", javaFiles...)
	output, err := cmd.CombinedOutput()
	if err != nil {
		t.Fatalf("javac failed: %v\n%s\n%s", err, text, string(output))
	}
}

func TestGenerateAddsJavaMainBridgeForTopLevelMain(t *testing.T) {
	src := `
def main() Int {
	0
}
`

	lowered := lowerProgram(t, src)
	source, err := GenerateForPackage(lowered, "")
	if err != nil {
		t.Fatalf("Generate returned error: %v", err)
	}

	text := string(source)
	if !strings.Contains(text, "public static long main()") {
		t.Fatalf("expected source main function in generated Java, got:\n%s", text)
	}
	if !strings.Contains(text, "public static void main(String[] args)") {
		t.Fatalf("expected Java entry bridge in generated Java, got:\n%s", text)
	}
	if !strings.Contains(text, "main();") {
		t.Fatalf("expected Java entry bridge to call top-level main(), got:\n%s", text)
	}
}

func TestGenerateCompilesWithTupleAndOptionRuntime(t *testing.T) {
	src := `
pair (Int, Str) = (1, "ok")

def run() Int {
	value Option[Int] = Some(5)
	return value.getOr(0)
}
`

	lowered := lowerProgram(t, src)
	source, err := GenerateForPackage(lowered, "")
	if err != nil {
		t.Fatalf("Generate returned error: %v", err)
	}

	text := string(source)
	if !strings.Contains(text, "new Tuple2<Long, String>(1L, \"ok\")") {
		t.Fatalf("expected tuple literal lowering in generated Java, got:\n%s", text)
	}
	if !strings.Contains(text, "Option.Some(5L)") {
		t.Fatalf("expected Some(...) lowering in generated Java, got:\n%s", text)
	}
	if !strings.Contains(text, "Option<Long>") {
		t.Fatalf("expected Option[T] Java type in generated Java, got:\n%s", text)
	}

	tmpDir := t.TempDir()
	if err := WriteStdlibSupport(tmpDir); err != nil {
		t.Fatalf("WriteStdlibSupport returned error: %v", err)
	}
	path := filepath.Join(tmpDir, "Pkg_Default.java")
	if err := os.WriteFile(path, source, 0o644); err != nil {
		t.Fatalf("write generated source: %v", err)
	}

	javaFiles := collectJavaFiles(t, tmpDir)
	cmd := exec.Command("javac", javaFiles...)
	output, err := cmd.CombinedOutput()
	if err != nil {
		t.Fatalf("javac failed: %v\n%s\n%s", err, text, string(output))
	}
}

func TestGenerateCompilesWithEitherRuntime(t *testing.T) {
	src := `
def make() Either[Str, Int] = Right(5)

def run() Int {
	make()
	return 0
}
`

	lowered := lowerProgram(t, src)
	source, err := GenerateForPackage(lowered, "")
	if err != nil {
		t.Fatalf("Generate returned error: %v", err)
	}

	text := string(source)
	if !strings.Contains(text, "Either.Right(5L)") {
		t.Fatalf("expected Right(...) lowering in generated Java, got:\n%s", text)
	}
	if !strings.Contains(text, "Either.Right(5L)") {
		t.Fatalf("expected Either runtime call in generated Java, got:\n%s", text)
	}

	tmpDir := t.TempDir()
	if err := WriteStdlibSupport(tmpDir); err != nil {
		t.Fatalf("WriteStdlibSupport returned error: %v", err)
	}
	path := filepath.Join(tmpDir, "Pkg_Default.java")
	if err := os.WriteFile(path, source, 0o644); err != nil {
		t.Fatalf("write generated source: %v", err)
	}

	javaFiles := collectJavaFiles(t, tmpDir)
	cmd := exec.Command("javac", javaFiles...)
	output, err := cmd.CombinedOutput()
	if err != nil {
		t.Fatalf("javac failed: %v\n%s\n%s", err, text, string(output))
	}
}

func TestGenerateCompilesWithRangeLoop(t *testing.T) {
	src := `
def run(limit Int) Int {
	var total Int = 0
	for i <- Range(0, limit) {
		total += i
	}
	return total
}
`

	lowered := lowerProgram(t, src)
	source, err := GenerateForPackage(lowered, "")
	if err != nil {
		t.Fatalf("Generate returned error: %v", err)
	}

	text := string(source)
	if !strings.Contains(text, "for (long i : new IntRange(0L, limit))") {
		t.Fatalf("expected Range(...) foreach lowering in generated Java, got:\n%s", text)
	}

	tmpDir := t.TempDir()
	if err := WriteStdlibSupport(tmpDir); err != nil {
		t.Fatalf("WriteStdlibSupport returned error: %v", err)
	}
	path := filepath.Join(tmpDir, "Pkg_Default.java")
	if err := os.WriteFile(path, source, 0o644); err != nil {
		t.Fatalf("write generated source: %v", err)
	}

	javaFiles := collectJavaFiles(t, tmpDir)
	cmd := exec.Command("javac", javaFiles...)
	output, err := cmd.CombinedOutput()
	if err != nil {
		t.Fatalf("javac failed: %v\n%s\n%s", err, text, string(output))
	}
}

func TestGenerateAvoidsDoubleWrappedIfConditions(t *testing.T) {
	src := `
def run(nums List[Int]) Int {
	if nums[0] > 0 {
		return 1
	}
	return 0
}
`

	lowered := lowerProgram(t, src)
	source, err := GenerateForPackage(lowered, "")
	if err != nil {
		t.Fatalf("Generate returned error: %v", err)
	}

	text := string(source)
	if strings.Contains(text, "if ((nums.get(0L).expect() > 0L))") {
		t.Fatalf("expected single-wrapped if condition, got:\n%s", text)
	}
	if !strings.Contains(text, "if (nums.get(0L).expect() > 0L)") {
		t.Fatalf("expected simplified if condition, got:\n%s", text)
	}
}

func TestGeneratePreservesWhileLoop(t *testing.T) {
	src := `
def run(keys List[Int]) Int {
	while keys.size() != 0 {
		keys.remove(0)
	}
	return 0
}
`

	lowered := lowerProgram(t, src)
	source, err := GenerateForPackage(lowered, "")
	if err != nil {
		t.Fatalf("Generate returned error: %v", err)
	}

	text := string(source)
	if strings.Contains(text, "while (true)") {
		t.Fatalf("expected direct while loop in generated Java, got:\n%s", text)
	}
	if !strings.Contains(text, "while (keys.size() != 0L)") {
		t.Fatalf("expected direct while condition in generated Java, got:\n%s", text)
	}
}

func TestGenerateCompilesWithListSetAndLambda(t *testing.T) {
	src := `
def run() Int {
	values List[List[Int]] = List(
		List(1, 2),
		List(3, 4)
	)
	seen Set[Int] = Set()
	stack List[Int] = List(0)
	row = values[0]
	first = row[0]
	row.forEach(next -> stack.append(next))
	seen.add(stack.remove(stack.size() - 1).getOr(-1) + first)
	return seen.size()
}
`

	lowered := lowerProgram(t, src)
	source, err := GenerateForPackage(lowered, "")
	if err != nil {
		t.Fatalf("Generate returned error: %v", err)
	}

	text := string(source)
	if !strings.Contains(text, "List<List<Long>>") {
		t.Fatalf("expected List type lowering in generated Java, got:\n%s", text)
	}
	if !strings.Contains(text, "row.get(0L).expect()") {
		t.Fatalf("expected list indexing lowering in generated Java, got:\n%s", text)
	}
	if !strings.Contains(text, "row.forEach(next -> stack.append(next))") {
		t.Fatalf("expected lambda lowering in generated Java, got:\n%s", text)
	}

	tmpDir := t.TempDir()
	if err := WriteStdlibSupport(tmpDir); err != nil {
		t.Fatalf("WriteStdlibSupport returned error: %v", err)
	}
	path := filepath.Join(tmpDir, "Pkg_Default.java")
	if err := os.WriteFile(path, source, 0o644); err != nil {
		t.Fatalf("write generated source: %v", err)
	}

	javaFiles := collectJavaFiles(t, tmpDir)
	cmd := exec.Command("javac", javaFiles...)
	output, err := cmd.CombinedOutput()
	if err != nil {
		t.Fatalf("javac failed: %v\n%s\n%s", err, text, string(output))
	}
}

func TestGenerateCompilesWithListLiteral(t *testing.T) {
	src := `
def run() Int {
	values List[Int] = [1, 2, 3]
	return values.size()
}
`

	lowered := lowerProgram(t, src)
	source, err := GenerateForPackage(lowered, "")
	if err != nil {
		t.Fatalf("Generate returned error: %v", err)
	}
	text := string(source)
	if !strings.Contains(text, "List.of(1L, 2L, 3L)") {
		t.Fatalf("expected list literal lowering in generated Java, got:\n%s", text)
	}
}

func TestGenerateUsesInheritanceBasedEnumLayout(t *testing.T) {
	src := `
enum Color {
	color Str
	temperature Int

	def isReddish() Bool = temperature % 5 == 0

	case Black {
		color = "xxx"
		temperature = 1
	}
	case Red {
		color = "xxx2"
		temperature = 2
	}
}

enum OptionX[T] {
	case NoneX
	case SomeX {
		value T
	}
}
`

	lowered := lowerProgram(t, src)
	source, err := GenerateForPackage(lowered, "")
	if err != nil {
		t.Fatalf("Generate returned error: %v", err)
	}

	text := string(source)
	if !strings.Contains(text, "abstract class Color") {
		t.Fatalf("expected shared-field enum to lower to abstract class, got:\n%s", text)
	}
	if !strings.Contains(text, "final class Color_Black extends Color") {
		t.Fatalf("expected enum case subclass for shared-field enum, got:\n%s", text)
	}
	if !strings.Contains(text, "interface OptionX<T>") {
		t.Fatalf("expected memberless enum to lower to interface, got:\n%s", text)
	}
	if !strings.Contains(text, "final class OptionX_SomeX<T> implements OptionX<T>") {
		t.Fatalf("expected generic enum case to implement enum interface, got:\n%s", text)
	}
	if strings.Contains(text, "__tag") {
		t.Fatalf("did not expect old tag-based enum lowering in generated Java, got:\n%s", text)
	}
}

func TestGenerateCompilesStatementfulIfExpr(t *testing.T) {
	src := `
enum MaybeInt {
	case NoneX
	case SomeX {
		value Int
	}
}

def run(values List[MaybeInt]) List[Option[Int]] {
	values.map(item -> partial item {
		case SomeX(x) if x > 0 => {
			x * 10
		}
	})
}
`

	lowered := lowerProgram(t, src)
	source, err := GenerateForPackage(lowered, "")
	if err != nil {
		t.Fatalf("Generate returned error: %v", err)
	}

	text := string(source)
	if !strings.Contains(text, "if (item instanceof MaybeInt_SomeX)") {
		t.Fatalf("expected statementful if-expression lowering in generated Java, got:\n%s", text)
	}

	tmpDir := t.TempDir()
	if err := WriteStdlibSupport(tmpDir); err != nil {
		t.Fatalf("WriteStdlibSupport returned error: %v", err)
	}
	path := filepath.Join(tmpDir, "Pkg_Default.java")
	if err := os.WriteFile(path, source, 0o644); err != nil {
		t.Fatalf("write generated source: %v", err)
	}

	javaFiles := collectJavaFiles(t, tmpDir)
	cmd := exec.Command("javac", javaFiles...)
	output, err := cmd.CombinedOutput()
	if err != nil {
		t.Fatalf("javac failed: %v\n%s\n%s", err, text, string(output))
	}
}

func TestOutputPathUsesPackageStructure(t *testing.T) {
	path := OutputPath("bin/java/src", "model/pubdemo")
	expected := filepath.Join("bin/java/src", "model", "pubdemo", "Pkg_Model_pubdemo.java")
	if path != expected {
		t.Fatalf("expected output path %q, got %q", expected, path)
	}
}

func TestOutputPathUsesSourceFileNameWhenPackageMissing(t *testing.T) {
	path := OutputPathFor("bin/java/src", "", "/tmp/find_circle_num.al")
	expected := filepath.Join("bin/java/src", "Pkg_FindCircleNum.java")
	if path != expected {
		t.Fatalf("expected output path %q, got %q", expected, path)
	}
}

func TestWriteStdlibSupportCreatesOptionResultEitherAndTupleFiles(t *testing.T) {
	tmpDir := t.TempDir()
	if err := WriteStdlibSupport(tmpDir); err != nil {
		t.Fatalf("WriteStdlibSupport returned error: %v", err)
	}
	for _, rel := range []string{
		filepath.Join("alang", "stdlib", "Either.java"),
		filepath.Join("alang", "stdlib", "Either_Left.java"),
		filepath.Join("alang", "stdlib", "Either_Right.java"),
		filepath.Join("alang", "stdlib", "OS.java"),
		filepath.Join("alang", "stdlib", "Map.java"),
		filepath.Join("alang", "stdlib", "Option.java"),
		filepath.Join("alang", "stdlib", "Option_None.java"),
		filepath.Join("alang", "stdlib", "Option_Some.java"),
		filepath.Join("alang", "stdlib", "Result.java"),
		filepath.Join("alang", "stdlib", "Result_Err.java"),
		filepath.Join("alang", "stdlib", "Result_Ok.java"),
		filepath.Join("alang", "stdlib", "Set.java"),
		filepath.Join("alang", "stdlib", "Tuple2.java"),
		filepath.Join("alang", "stdlib", "Tuple10.java"),
	} {
		if _, err := os.Stat(filepath.Join(tmpDir, rel)); err != nil {
			t.Fatalf("expected runtime file %s: %v", rel, err)
		}
	}
}

func TestOptionJavaSourceFromPredef(t *testing.T) {
	registry, err := predef.Load()
	if err != nil {
		t.Fatalf("predef.Load returned error: %v", err)
	}
	text, err := optionJavaSourceFromPredef(registry)
	if err != nil {
		t.Fatalf("optionJavaSourceFromPredef returned error: %v", err)
	}
	if !strings.Contains(text, "interface Option<") && !strings.Contains(text, "abstract class Option<") {
		t.Fatalf("expected generated Option type, got:\n%s", text)
	}
}

func TestResultJavaSourceFromPredef(t *testing.T) {
	registry, err := predef.Load()
	if err != nil {
		t.Fatalf("predef.Load returned error: %v", err)
	}
	sources, err := resultJavaSourcesFromPredef(registry)
	if err != nil {
		t.Fatalf("resultJavaSourcesFromPredef returned error: %v", err)
	}
	text := sources[filepath.Join("alang", "stdlib", "Result.java")]
	if !strings.Contains(text, "interface Result<") && !strings.Contains(text, "abstract class Result<") {
		t.Fatalf("expected generated Result type, got:\n%s", text)
	}
	if !strings.Contains(text, "static <T, E> Result<T, E> Ok(T value)") {
		t.Fatalf("expected generated Result.Ok factory, got:\n%s", text)
	}
}

func TestWriteStdlibSupportGeneratesTupleFromPredefShape(t *testing.T) {
	tmpDir := t.TempDir()
	if err := WriteStdlibSupport(tmpDir); err != nil {
		t.Fatalf("WriteStdlibSupport returned error: %v", err)
	}
	bytes, err := os.ReadFile(filepath.Join(tmpDir, "alang", "stdlib", "Tuple2.java"))
	if err != nil {
		t.Fatalf("read generated Tuple2 runtime: %v", err)
	}
	text := string(bytes)
	if !strings.Contains(text, "public final A _1;") || !strings.Contains(text, "public final B _2;") {
		t.Fatalf("expected Tuple2 runtime to follow predef field names, got:\n%s", text)
	}
}

func TestWriteStdlibSupportCopiesBundledListRuntime(t *testing.T) {
	tmpDir := t.TempDir()
	if err := WriteStdlibSupport(tmpDir); err != nil {
		t.Fatalf("WriteStdlibSupport returned error: %v", err)
	}
	bytes, err := os.ReadFile(filepath.Join(tmpDir, "alang", "stdlib", "List.java"))
	if err != nil {
		t.Fatalf("read copied List runtime: %v", err)
	}
	text := string(bytes)
	if !strings.Contains(text, "public <X> List<X> map(") {
		t.Fatalf("expected checked-in List runtime contents, got:\n%s", text)
	}
	if !strings.Contains(text, "public Option<T> removeLast()") {
		t.Fatalf("expected richer List runtime contents, got:\n%s", text)
	}
}

func TestWriteStdlibSupportCopiesBundledSetAndMapRuntime(t *testing.T) {
	tmpDir := t.TempDir()
	if err := WriteStdlibSupport(tmpDir); err != nil {
		t.Fatalf("WriteStdlibSupport returned error: %v", err)
	}

	setBytes, err := os.ReadFile(filepath.Join(tmpDir, "alang", "stdlib", "Set.java"))
	if err != nil {
		t.Fatalf("read copied Set runtime: %v", err)
	}
	setText := string(setBytes)
	if !strings.Contains(setText, "private final HashSet<T> items;") {
		t.Fatalf("expected checked-in Set runtime contents, got:\n%s", setText)
	}
	if !strings.Contains(setText, "public <X> Set<X> map(") {
		t.Fatalf("expected richer Set runtime contents, got:\n%s", setText)
	}

	mapBytes, err := os.ReadFile(filepath.Join(tmpDir, "alang", "stdlib", "Map.java"))
	if err != nil {
		t.Fatalf("read copied Map runtime: %v", err)
	}
	mapText := string(mapBytes)
	if !strings.Contains(mapText, "private final HashMap<K, V> items;") {
		t.Fatalf("expected checked-in Map runtime contents, got:\n%s", mapText)
	}
	if !strings.Contains(mapText, "public <X> Map<K, X> mapValues(") {
		t.Fatalf("expected richer Map runtime contents, got:\n%s", mapText)
	}
}

func TestGenerateFindCircleNumCompilesWithJavac(t *testing.T) {
	srcBytes, err := os.ReadFile(filepath.Join("..", "..", "examples", "random_code", "find_circle_num.al"))
	if err != nil {
		t.Fatalf("read example source: %v", err)
	}

	lowered := lowerProgram(t, string(srcBytes))
	source, err := GenerateForPackage(lowered, "")
	if err != nil {
		t.Fatalf("Generate returned error: %v", err)
	}

	tmpDir := t.TempDir()
	if err := WriteStdlibSupport(tmpDir); err != nil {
		t.Fatalf("WriteStdlibSupport returned error: %v", err)
	}
	path := filepath.Join(tmpDir, "Pkg_Default.java")
	if err := os.WriteFile(path, source, 0o644); err != nil {
		t.Fatalf("write generated source: %v", err)
	}

	javaFiles := collectJavaFiles(t, tmpDir)
	cmd := exec.Command("javac", javaFiles...)
	output, err := cmd.CombinedOutput()
	if err != nil {
		t.Fatalf("javac failed: %v\n%s\n%s", err, string(source), string(output))
	}
}

func collectJavaFiles(t *testing.T, root string) []string {
	t.Helper()
	var files []string
	err := filepath.Walk(root, func(path string, info os.FileInfo, err error) error {
		if err != nil {
			return err
		}
		if info.IsDir() || filepath.Ext(path) != ".java" {
			return nil
		}
		files = append(files, path)
		return nil
	})
	if err != nil {
		t.Fatalf("collect Java files: %v", err)
	}
	return files
}
