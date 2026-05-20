package predef

import (
	"bufio"
	"fmt"
	"os"
	"path/filepath"
	"runtime"
	"sort"
	"strings"
	"sync"

	"a-lang/parser"
)

type Kind string

const (
	KindInterface Kind = "interface"
	KindClass     Kind = "class"
	KindObject    Kind = "object"
	KindRecord    Kind = "record"
	KindEnum      Kind = "enum"
)

type MethodDescriptor struct {
	Name           string
	TypeParameters []parser.TypeParameter
	Parameters     []parser.Parameter
	ReturnType     *parser.TypeRef
	Private        bool
	Operator       bool
	Constructor    bool
}

type TypeDescriptor struct {
	Name                  string
	Kind                  Kind
	TypeParameters        []parser.TypeParameter
	Fields                []parser.FieldDecl
	Methods               []MethodDescriptor
	ImplementedInterfaces []*parser.TypeRef
	Directives            FileDirectives
}

type FileDirectives struct {
	PreludeSkip bool
	JavaSkip    bool
	Interpreter bool
}

type Registry struct {
	Program    *parser.Program
	Types      map[string]TypeDescriptor
	Classes    map[string]*parser.ClassDecl
	Interfaces map[string]*parser.InterfaceDecl
}

var (
	loadOnce sync.Once
	loaded   *Registry
	loadErr  error
)

func Load() (*Registry, error) {
	loadOnce.Do(func() {
		loaded, loadErr = load()
	})
	return loaded, loadErr
}

func load() (*Registry, error) {
	dir, err := predefDir()
	if err != nil {
		return nil, err
	}
	entries, err := os.ReadDir(dir)
	if err != nil {
		return nil, err
	}
	var paths []string
	for _, entry := range entries {
		if entry.IsDir() || filepath.Ext(entry.Name()) != ".lum" {
			continue
		}
		paths = append(paths, filepath.Join(dir, entry.Name()))
	}
	sort.Strings(paths)

	registry := &Registry{
		Program:    &parser.Program{},
		Types:      map[string]TypeDescriptor{},
		Classes:    map[string]*parser.ClassDecl{},
		Interfaces: map[string]*parser.InterfaceDecl{},
	}
	for _, path := range paths {
		directives, err := ReadDirectives(path)
		if err != nil {
			return nil, err
		}
		src, err := os.ReadFile(path)
		if err != nil {
			return nil, err
		}
		program, err := parser.Parse(string(src))
		if err != nil {
			return nil, fmt.Errorf("parse predef %q: %w", filepath.Base(path), err)
		}
		registry.Program.Functions = append(registry.Program.Functions, program.Functions...)
		registry.Program.Interfaces = append(registry.Program.Interfaces, program.Interfaces...)
		registry.Program.Classes = append(registry.Program.Classes, program.Classes...)

		for _, decl := range program.Interfaces {
			registry.Interfaces[decl.Name] = decl
			methods := make([]MethodDescriptor, len(decl.Methods))
			for i, method := range decl.Methods {
				methods[i] = MethodDescriptor{
					Name:           method.Name,
					TypeParameters: append([]parser.TypeParameter(nil), method.TypeParameters...),
					Parameters:     append([]parser.Parameter(nil), method.Parameters...),
					ReturnType:     method.ReturnType,
				}
			}
			registry.Types[decl.Name] = TypeDescriptor{
				Name:                  decl.Name,
				Kind:                  KindInterface,
				TypeParameters:        append([]parser.TypeParameter(nil), decl.TypeParameters...),
				Methods:               methods,
				ImplementedInterfaces: append([]*parser.TypeRef(nil), decl.Extends...),
				Directives:            directives,
			}
		}
		for _, decl := range program.Classes {
			registry.Classes[decl.Name] = decl
			kind := KindClass
			switch {
			case decl.Object:
				kind = KindObject
			case decl.Record:
				kind = KindRecord
			case decl.Enum:
				kind = KindEnum
			}
			methods := make([]MethodDescriptor, len(decl.Methods))
			for i, method := range decl.Methods {
				methods[i] = MethodDescriptor{
					Name:           method.Name,
					TypeParameters: append([]parser.TypeParameter(nil), method.TypeParameters...),
					Parameters:     append([]parser.Parameter(nil), method.Parameters...),
					ReturnType:     method.ReturnType,
					Private:        method.Private,
					Operator:       method.Operator,
					Constructor:    method.Constructor,
				}
			}
			registry.Types[decl.Name] = TypeDescriptor{
				Name:                  decl.Name,
				Kind:                  kind,
				TypeParameters:        append([]parser.TypeParameter(nil), decl.TypeParameters...),
				Fields:                append([]parser.FieldDecl(nil), decl.Fields...),
				Methods:               methods,
				ImplementedInterfaces: append([]*parser.TypeRef(nil), decl.Implements...),
				Directives:            directives,
			}
		}
	}
	return registry, nil
}

func predefDir() (string, error) {
	_, file, _, ok := runtime.Caller(0)
	if !ok {
		return "", fmt.Errorf("resolve predef directory: runtime caller unavailable")
	}
	return filepath.Join(filepath.Dir(file), "..", "stdlib"), nil
}

func ReadDirectives(path string) (FileDirectives, error) {
	file, err := os.Open(path)
	if err != nil {
		return FileDirectives{}, err
	}
	defer file.Close()

	var directives FileDirectives
	scanner := bufio.NewScanner(file)
	for scanner.Scan() {
		line := strings.TrimSpace(scanner.Text())
		if line == "" {
			continue
		}
		if !strings.HasPrefix(line, "#") {
			break
		}
		switch line {
		case "# SKIP", "# PRELUDE_SKIP":
			directives.PreludeSkip = true
		case "# JAVA_SKIP":
			directives.JavaSkip = true
		case "# INTERPRETER":
			directives.Interpreter = true
		}
	}
	if err := scanner.Err(); err != nil {
		return FileDirectives{}, err
	}
	return directives, nil
}
