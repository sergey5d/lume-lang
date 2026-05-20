package examples_test

import (
	"a-lang/interpreter"
	"a-lang/module"
	"a-lang/semantic"
	"a-lang/typecheck"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"regexp"
	"sort"
	"strings"
	"testing"
)

func TestExamples(t *testing.T) {
	var paths []string
	err := filepath.WalkDir(".", func(path string, d os.DirEntry, err error) error {
		if err != nil {
			return err
		}
		if d.IsDir() {
			return nil
		}
		if filepath.Ext(path) == ".lum" {
			paths = append(paths, path)
		}
		return nil
	})
	if err != nil {
		t.Fatalf("WalkDir returned error: %v", err)
	}
	sort.Strings(paths)
	if len(paths) == 0 {
		t.Fatalf("expected at least one example in examples/")
	}

	for _, path := range paths {
		path := path
		name := strings.TrimPrefix(filepath.ToSlash(path), "./")
		srcBytes, err := os.ReadFile(path)
		if err != nil {
			t.Fatalf("ReadFile returned error for %s: %v", name, err)
		}
		src := string(srcBytes)
		if shouldSkipExample(src) {
			continue
		}
		t.Run(name, func(t *testing.T) {
			expectedOutput, outputErr := parseExpectedOutput(src)
			expectedFailure, failureErr := parseExpectedFailure(src)
			expectedFailureRegex, failureRegexErr := parseExpectedFailureRegex(src)
			switch {
			case countDefined(outputErr == nil, failureErr == nil, failureRegexErr == nil) > 1:
				t.Fatalf("FAILURE in %s\nexample cannot declare more than one of # EXPECT, # FAIL, or # FAIL_REGEX", name)
			case outputErr == errMissingExpectedHeader && failureErr == errMissingFailureHeader && failureRegexErr == errMissingFailureRegexHeader:
				t.Skipf("skipping %s: no # EXPECT, # FAIL, or # FAIL_REGEX header", name)
			case outputErr != nil && outputErr != errMissingExpectedHeader:
				t.Fatalf("FAILURE in %s\nparseExpectedOutput returned error: %v", name, outputErr)
			case failureErr != nil && failureErr != errMissingFailureHeader:
				t.Fatalf("FAILURE in %s\nparseExpectedFailure returned error: %v", name, failureErr)
			case failureRegexErr != nil && failureRegexErr != errMissingFailureRegexHeader:
				t.Fatalf("FAILURE in %s\nparseExpectedFailureRegex returned error: %v", name, failureRegexErr)
			}

			if failureErr == nil {
				actualFailure := runExampleFailure(path)
				if normalizeExampleOutput(actualFailure) != normalizeExampleOutput(expectedFailure) {
					t.Fatalf("FAILURE in %s\nexpected failure:\n%s\nactual failure:\n%s", name, expectedFailure, actualFailure)
				}
				return
			}
			if failureRegexErr == nil {
				actualFailure := normalizeExampleOutput(runExampleFailure(path))
				pattern := "(?s)^" + expectedFailureRegex + "$"
				matched, err := regexp.MatchString(pattern, actualFailure)
				if err != nil {
					t.Fatalf("FAILURE in %s\ninvalid failure regex: %v", name, err)
				}
				if !matched {
					t.Fatalf("FAILURE in %s\nexpected failure regex:\n%s\nactual failure:\n%s", name, expectedFailureRegex, actualFailure)
				}
				return
			}

			expected := expectedOutput
			if outputErr != nil {
				if outputErr == errMissingExpectedHeader {
					t.Skipf("skipping %s: no # EXPECT header", name)
				}
				t.Fatalf("FAILURE in %s\nparseExpectedOutput returned error: %v", name, outputErr)
			}

			loaded, err := module.Load(path)
			if err != nil {
				t.Fatalf("FAILURE in %s\nload error: %v", name, err)
			}
			diagnostics := semantic.AnalyzeModule(loaded)
			typeResult := typecheck.AnalyzeModule(loaded)
			diagnostics = append(diagnostics, typeResult.Diagnostics...)
			if len(diagnostics) > 0 {
				var messages []string
				for _, diagnostic := range diagnostics {
					messages = append(messages, diagnostic.Error())
				}
				t.Fatalf("FAILURE in %s\ndiagnostics:\n%s", name, strings.Join(messages, "\n"))
			}

			oldStdout := os.Stdout
			reader, writer, err := os.Pipe()
			if err != nil {
				t.Fatalf("Pipe returned error: %v", err)
			}
			os.Stdout = writer

			in := interpreter.NewModule(loaded)
			value, callErr := in.Call("main")

			_ = writer.Close()
			os.Stdout = oldStdout
			output, _ := io.ReadAll(reader)
			_ = reader.Close()

			if callErr != nil {
				t.Fatalf("example %s runtime failure: %v", name, callErr)
			}

			actual := string(output)
			if value != nil {
				actual += fmt.Sprintln(value)
			}

			if normalizeExampleOutput(actual) != normalizeExampleOutput(expected) {
				t.Fatalf("FAILURE in %s\nunexpected output\nexpected:\n%s\nactual:\n%s", name, expected, actual)
			}
		})
	}
}

var errMissingExpectedHeader = fmt.Errorf("missing '# EXPECT:' header")
var errMissingFailureHeader = fmt.Errorf("missing '# FAIL:' header")
var errMissingFailureRegexHeader = fmt.Errorf("missing '# FAIL_REGEX:' header")

func parseExpectedOutput(src string) (string, error) {
	return parseCommentBlock(src, "# EXPECT:")
}

func parseExpectedFailure(src string) (string, error) {
	return parseCommentBlock(src, "# FAIL:")
}

func parseExpectedFailureRegex(src string) (string, error) {
	return parseCommentBlock(src, "# FAIL_REGEX:")
}

func parseCommentBlock(src, header string) (string, error) {
	lines := strings.Split(src, "\n")
	start := -1
	for i, line := range lines {
		trimmed := strings.TrimSpace(line)
		if trimmed == header {
			start = i + 1
			break
		}
		if trimmed != "" && !strings.HasPrefix(trimmed, "#") {
			break
		}
	}
	if start == -1 {
		switch header {
		case "# EXPECT:":
			return "", errMissingExpectedHeader
		case "# FAIL:":
			return "", errMissingFailureHeader
		case "# FAIL_REGEX:":
			return "", errMissingFailureRegexHeader
		default:
			return "", fmt.Errorf("missing %q header", header)
		}
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
	return strings.Join(out, "\n"), nil
}

func countDefined(values ...bool) int {
	count := 0
	for _, value := range values {
		if value {
			count++
		}
	}
	return count
}

func runExampleFailure(path string) string {
	loaded, err := module.Load(path)
	if err != nil {
		return err.Error()
	}
	diagnostics := semantic.AnalyzeModule(loaded)
	typeResult := typecheck.AnalyzeModule(loaded)
	diagnostics = append(diagnostics, typeResult.Diagnostics...)
	if len(diagnostics) > 0 {
		var messages []string
		seen := map[string]bool{}
		for _, diagnostic := range diagnostics {
			message := diagnostic.Error()
			if seen[message] {
				continue
			}
			seen[message] = true
			messages = append(messages, message)
		}
		return strings.Join(messages, "\n")
	}

	in := interpreter.NewModule(loaded)
	if _, err := in.Call("main"); err != nil {
		return err.Error()
	}
	return "expected example to fail, but it succeeded"
}

func normalizeExampleOutput(s string) string {
	return strings.TrimRight(s, "\n")
}

func shouldSkipExample(src string) bool {
	for _, line := range strings.Split(src, "\n") {
		trimmed := strings.TrimSpace(line)
		if trimmed == "" {
			continue
		}
		if trimmed == "# SKIP" || strings.HasPrefix(trimmed, "# SKIP:") {
			return true
		}
		if !strings.HasPrefix(trimmed, "#") {
			return false
		}
	}
	return false
}
