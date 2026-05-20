package main

import (
	"a-lang/codegen/java"
	"a-lang/interpreter"
	"a-lang/lower"
	"a-lang/module"
	"a-lang/parser"
	"a-lang/semantic"
	"a-lang/typecheck"
	"a-lang/typed"
	"encoding/json"
	"fmt"
	"io"
	"io/fs"
	"os"
	"path/filepath"
	"strconv"
	"strings"
)

func main() {
	if len(os.Args) < 2 {
		fmt.Fprintln(os.Stderr, "usage: lume <file> [run [entry [args...]]|ast|java]")
		os.Exit(1)
	}

	mode := "run"
	if len(os.Args) >= 3 {
		mode = os.Args[2]
	}

	srcBytes, readErr := os.ReadFile(os.Args[1])
	src := ""
	if readErr == nil {
		src = string(srcBytes)
	}

	if mode == "java" {
		if err := java.WriteStdlibSupport("bin/java/stdlib/src"); err != nil {
			fmt.Fprintf(os.Stderr, "write java stdlib support: %v\n", err)
			os.Exit(1)
		}
		info, err := os.Stat(os.Args[1])
		if err != nil {
			fmt.Fprintf(os.Stderr, "stat source: %v\n", err)
			os.Exit(1)
		}
		if info.IsDir() {
			if err := generateJavaTree(os.Args[1]); err != nil {
				fmt.Fprintln(os.Stderr, err)
				os.Exit(1)
			}
			return
		}
		if readErr != nil {
			fmt.Fprintf(os.Stderr, "read source: %v\n", readErr)
			os.Exit(1)
		}
		program, err := parser.Parse(src)
		if err != nil {
			fmt.Fprintf(os.Stderr, "parse error: %v\n", err)
			os.Exit(1)
		}
		if len(program.Imports) > 0 {
			fmt.Fprintln(os.Stderr, "java generation does not support imports/modules yet")
			os.Exit(1)
		}
		diagnostics := semantic.Analyze(program)
		typeResult := typecheck.Analyze(program)
		diagnostics = append(diagnostics, typeResult.Diagnostics...)
		if len(diagnostics) > 0 {
			seen := map[string]bool{}
			for _, diagnostic := range diagnostics {
				message := formatDiagnostic(diagnostic, src)
				if seen[message] {
					continue
				}
				seen[message] = true
				fmt.Fprintln(os.Stderr, message)
			}
			os.Exit(1)
		}
		typedProgram, err := typed.Build(program, typeResult)
		if err != nil {
			fmt.Fprintf(os.Stderr, "typed build: %v\n", err)
			os.Exit(1)
		}
		lowered, err := lower.ProgramFromTyped(typedProgram)
		if err != nil {
			fmt.Fprintf(os.Stderr, "lowering: %v\n", err)
			os.Exit(1)
		}
		generated, err := java.GenerateForPackageSource(lowered, program.PackageName, os.Args[1])
		if err != nil {
			fmt.Fprintf(os.Stderr, "java generation: %v\n", err)
			os.Exit(1)
		}
		outputPath := java.OutputPathFor("bin/java/src", program.PackageName, os.Args[1])
		if err := os.MkdirAll(filepath.Dir(outputPath), 0o755); err != nil {
			fmt.Fprintf(os.Stderr, "create java output dir: %v\n", err)
			os.Exit(1)
		}
		if err := os.WriteFile(outputPath, generated, 0o644); err != nil {
			fmt.Fprintf(os.Stderr, "write java output: %v\n", err)
			os.Exit(1)
		}
		fmt.Print(string(generated))
		return
	}

	loaded, err := module.Load(os.Args[1])
	if err != nil {
		fmt.Fprintf(os.Stderr, "load error: %v\n", err)
		os.Exit(1)
	}
	program := loaded.Program

	diagnostics := semantic.AnalyzeModule(loaded)
	typeResult := typecheck.AnalyzeModule(loaded)
	diagnostics = append(diagnostics, typeResult.Diagnostics...)
	if len(diagnostics) > 0 {
		seen := map[string]bool{}
		for _, diagnostic := range diagnostics {
			message := formatDiagnostic(diagnostic, src)
			if seen[message] {
				continue
			}
			seen[message] = true
			fmt.Fprintln(os.Stderr, message)
		}
		os.Exit(1)
	}

	switch mode {
	case "ast":
		out, err := json.MarshalIndent(program, "", "  ")
		if err != nil {
			fmt.Fprintf(os.Stderr, "marshal ast: %v\n", err)
			os.Exit(1)
		}
		fmt.Println(string(out))
	case "run":
		entry := "main"
		rawArgs := []string{}
		if len(os.Args) >= 4 {
			entry = os.Args[3]
			rawArgs = os.Args[4:]
		}
		entryFn := findFunction(program, entry)
		if entryFn == nil {
			fmt.Fprintf(os.Stderr, "unknown entry %q\n", entry)
			os.Exit(1)
		}
		args, err := parseCLIArgs(entryFn, rawArgs)
		if err != nil {
			fmt.Fprintln(os.Stderr, err)
			os.Exit(1)
		}
		in := interpreter.NewModule(loaded)
		if readErr != nil {
			fmt.Fprintf(os.Stderr, "read source: %v\n", readErr)
			os.Exit(1)
		}
		expected, hasExpected, err := parseExpectedOutput(string(srcBytes))
		if err != nil {
			fmt.Fprintf(os.Stderr, "parse expected output: %v\n", err)
			os.Exit(1)
		}
		if !hasExpected {
			value, err := in.Call(entry, args...)
			if err != nil {
				fmt.Fprintln(os.Stderr, formatRuntimeError(err, src))
				os.Exit(1)
			}
			if value != nil {
				fmt.Println(value)
			}
			return
		}

		oldStdout := os.Stdout
		reader, writer, err := os.Pipe()
		if err != nil {
			fmt.Fprintf(os.Stderr, "pipe: %v\n", err)
			os.Exit(1)
		}
		os.Stdout = writer
		value, err := in.Call(entry, args...)
		_ = writer.Close()
		os.Stdout = oldStdout
		output, _ := io.ReadAll(reader)
		_ = reader.Close()
		if err != nil {
			fmt.Fprintln(os.Stderr, formatRuntimeError(err, src))
			os.Exit(1)
		}
		actual := string(output)
		if value != nil {
			actual += fmt.Sprintln(value)
		}
		if normalizeExampleOutput(actual) != normalizeExampleOutput(expected) {
			fmt.Fprintf(os.Stderr, "example output mismatch\nexpected:\n%s\nactual:\n%s", expected, actual)
			os.Exit(1)
		}
		fmt.Print(actual)
	default:
		fmt.Fprintf(os.Stderr, "unknown mode %q\n", mode)
		os.Exit(1)
	}
}

func generateJavaTree(root string) error {
	written := map[string]string{}
	var failures []string
	err := filepath.WalkDir(root, func(path string, d fs.DirEntry, err error) error {
		if err != nil {
			failures = append(failures, fmt.Sprintf("%s: %v", path, err))
			return nil
		}
		if d.IsDir() {
			return nil
		}
		if filepath.Ext(path) != ".lum" {
			return nil
		}
		generated, outputPath, err := generateJavaFile(path)
		if err != nil {
			failures = append(failures, fmt.Sprintf("%s: %v", path, err))
			return nil
		}
		if prior, ok := written[outputPath]; ok {
			failures = append(failures, fmt.Sprintf("java output collision: %s and %s both map to %s", prior, path, outputPath))
			return nil
		}
		written[outputPath] = path
		if err := os.MkdirAll(filepath.Dir(outputPath), 0o755); err != nil {
			failures = append(failures, fmt.Sprintf("%s: create java output dir: %v", path, err))
			return nil
		}
		if err := os.WriteFile(outputPath, generated, 0o644); err != nil {
			failures = append(failures, fmt.Sprintf("%s: write java output: %v", path, err))
			return nil
		}
		fmt.Printf("generated %s -> %s\n", path, outputPath)
		return nil
	})
	if err != nil {
		return err
	}
	if len(failures) == 0 {
		return nil
	}
	return fmt.Errorf("java generation completed with %d failure(s):\n%s", len(failures), strings.Join(failures, "\n"))
}

func generateJavaFile(path string) ([]byte, string, error) {
	srcBytes, err := os.ReadFile(path)
	if err != nil {
		return nil, "", fmt.Errorf("read source: %w", err)
	}
	src := string(srcBytes)
	program, err := parser.Parse(src)
	if err != nil {
		return nil, "", fmt.Errorf("parse error: %w", err)
	}
	if len(program.Imports) > 0 {
		return nil, "", fmt.Errorf("java generation does not support imports/modules yet")
	}
	diagnostics := semantic.Analyze(program)
	typeResult := typecheck.Analyze(program)
	diagnostics = append(diagnostics, typeResult.Diagnostics...)
	if len(diagnostics) > 0 {
		var parts []string
		seen := map[string]bool{}
		for _, diagnostic := range diagnostics {
			message := formatDiagnostic(diagnostic, src)
			if seen[message] {
				continue
			}
			seen[message] = true
			parts = append(parts, message)
		}
		return nil, "", fmt.Errorf("%s", strings.Join(parts, "\n"))
	}
	typedProgram, err := typed.Build(program, typeResult)
	if err != nil {
		return nil, "", fmt.Errorf("typed build: %w", err)
	}
	lowered, err := lower.ProgramFromTyped(typedProgram)
	if err != nil {
		return nil, "", fmt.Errorf("lowering: %w", err)
	}
	generated, err := java.GenerateForPackageSource(lowered, program.PackageName, path)
	if err != nil {
		return nil, "", fmt.Errorf("java generation: %w", err)
	}
	return generated, java.OutputPathFor("bin/java/src", program.PackageName, path), nil
}

func formatDiagnostic(d semantic.Diagnostic, src string) string {
	return formatSpanMessage(d.Error(), d.Span, src)
}

func formatRuntimeError(err error, src string) string {
	runtimeErr, ok := err.(interpreter.RuntimeError)
	if !ok {
		return err.Error()
	}
	return formatSpanMessage(runtimeErr.Error(), runtimeErr.Span, src)
}

func formatSpanMessage(message string, span parser.Span, src string) string {
	lines := strings.Split(src, "\n")
	line := span.Start.Line
	if line <= 0 || line > len(lines) {
		return message
	}

	startLine := maxInt(1, line-1)
	endLine := minInt(len(lines), line+1)
	gutterWidth := len(strconv.Itoa(endLine))
	target := lines[line-1]

	startCol := span.Start.Column
	if startCol <= 0 {
		startCol = 1
	}
	if startCol > len([]rune(target))+1 {
		startCol = len([]rune(target)) + 1
	}
	underlineLen := 1
	if span.End.Line == span.Start.Line && span.End.Column > span.Start.Column {
		underlineLen = span.End.Column - span.Start.Column
	}
	if underlineLen <= 0 {
		underlineLen = 1
	}

	var b strings.Builder
	b.WriteString(message)
	b.WriteString("\n\n")
	for i := startLine; i <= endLine; i++ {
		fmt.Fprintf(&b, "%*d │ %s\n", gutterWidth, i, lines[i-1])
		if i == line {
			fmt.Fprintf(&b, "%s │ %s%s\n", strings.Repeat(" ", gutterWidth), strings.Repeat(" ", startCol-1), strings.Repeat("^", underlineLen))
		}
	}
	return strings.TrimRight(b.String(), "\n")
}

func minInt(a, b int) int {
	if a < b {
		return a
	}
	return b
}

func maxInt(a, b int) int {
	if a > b {
		return a
	}
	return b
}

func parseExpectedOutput(src string) (string, bool, error) {
	lines := strings.Split(src, "\n")
	start := -1
	for i, line := range lines {
		trimmed := strings.TrimSpace(line)
		if trimmed == "# EXPECT:" {
			start = i + 1
			break
		}
		if trimmed != "" && !strings.HasPrefix(trimmed, "#") {
			return "", false, nil
		}
	}
	if start == -1 {
		return "", false, nil
	}

	var out []string
	for i := start; i < len(lines); i++ {
		trimmed := strings.TrimSpace(lines[i])
		if !strings.HasPrefix(trimmed, "#") {
			break
		}
		content := strings.TrimPrefix(trimmed, "#")
		if strings.HasPrefix(content, " ") {
			content = content[1:]
		}
		out = append(out, content)
	}
	return strings.Join(out, "\n"), true, nil
}

func normalizeExampleOutput(s string) string {
	return strings.TrimRight(s, "\n")
}

func findFunction(program *parser.Program, name string) *parser.FunctionDecl {
	for _, fn := range program.Functions {
		if fn.Name == name {
			return fn
		}
	}
	return nil
}

func parseCLIArgs(fn *parser.FunctionDecl, raw []string) ([]interpreter.Value, error) {
	if len(raw) != len(fn.Parameters) {
		return nil, fmt.Errorf("entry %q expects %d arguments, got %d", fn.Name, len(fn.Parameters), len(raw))
	}
	args := make([]interpreter.Value, len(raw))
	for i, param := range fn.Parameters {
		value, err := parseCLIArg(param.Type, raw[i])
		if err != nil {
			return nil, fmt.Errorf("argument %d (%s): %w", i+1, param.Name, err)
		}
		args[i] = value
	}
	return args, nil
}

func parseCLIArg(ref *parser.TypeRef, raw string) (interpreter.Value, error) {
	if ref == nil {
		return nil, fmt.Errorf("missing parameter type")
	}
	switch ref.Name {
	case "Int", "Int64":
		value, err := strconv.ParseInt(raw, 10, 64)
		if err != nil {
			return nil, fmt.Errorf("expected Int, got %q", raw)
		}
		return value, nil
	case "Float", "Float64":
		value, err := strconv.ParseFloat(raw, 64)
		if err != nil {
			return nil, fmt.Errorf("expected Float, got %q", raw)
		}
		return value, nil
	case "Bool":
		value, err := strconv.ParseBool(raw)
		if err != nil {
			return nil, fmt.Errorf("expected Bool, got %q", raw)
		}
		return value, nil
	case "Str":
		return raw, nil
	case "Rune":
		runes := []rune(raw)
		if len(runes) != 1 {
			return nil, fmt.Errorf("expected Rune, got %q", raw)
		}
		return runes[0], nil
	default:
		return nil, fmt.Errorf("CLI arguments do not support type %s yet", ref.Name)
	}
}
