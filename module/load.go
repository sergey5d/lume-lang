package module

import (
	"fmt"
	"os"
	"path/filepath"
	"sort"

	"a-lang/parser"
	"a-lang/predef"
)

type LoadedModule struct {
	Path          string
	SourceProgram *parser.Program
	Program       *parser.Program
	Imports       map[string]*LoadedModule
	ImportPaths   map[string]string
	SymbolImports map[string]ImportedSymbol
	Dependencies  map[string]*LoadedModule
}

type ImportedSymbol struct {
	LocalName    string
	OriginalName string
	ObjectName   string
	IsInterface  bool
	IsFunction   bool
	IsValue      bool
	Module       *LoadedModule
}

func Load(path string) (*LoadedModule, error) {
	cache := map[string]*LoadedModule{}
	loading := map[string]bool{}
	return load(path, cache, loading)
}

func load(path string, cache map[string]*LoadedModule, loading map[string]bool) (*LoadedModule, error) {
	abs, err := filepath.Abs(path)
	if err != nil {
		return nil, err
	}
	if mod, ok := cache[abs]; ok {
		return mod, nil
	}
	if loading[abs] {
		return nil, fmt.Errorf("import cycle detected at %s", abs)
	}
	loading[abs] = true
	defer delete(loading, abs)

	src, err := os.ReadFile(abs)
	if err != nil {
		return nil, err
	}
	sourceProgram, err := parser.Parse(string(src))
	if err != nil {
		return nil, err
	}
	program := sourceProgram
	stdlibDir, _ := findStdlibDir(filepath.Dir(abs))
	if stdlibDir != "" {
		preludePrograms, err := loadPreludePrograms(stdlibDir, abs)
		if err != nil {
			return nil, err
		}
		program = mergePrelude(program, preludePrograms)
	}

	mod := &LoadedModule{
		Path:          abs,
		SourceProgram: sourceProgram,
		Program:       program,
		Imports:       map[string]*LoadedModule{},
		ImportPaths:   map[string]string{},
		SymbolImports: map[string]ImportedSymbol{},
		Dependencies:  map[string]*LoadedModule{},
	}
	cache[abs] = mod

	baseDir := filepath.Dir(abs)
	for _, imp := range program.Imports {
		childPath := filepath.Join(baseDir, filepath.FromSlash(imp.Path)+".lum")
		child, err := load(childPath, cache, loading)
		if err != nil {
			return nil, fmt.Errorf("load import %q: %w", imp.Path, err)
		}
		mod.Dependencies[child.Path] = child
		if imp.ObjectName == "" && len(imp.Symbols) == 0 && !imp.Wildcard {
			alias := filepath.Base(imp.Path)
			if existing, ok := mod.ImportPaths[alias]; ok && existing != imp.Path {
				return nil, fmt.Errorf("duplicate import alias '%s' for paths '%s' and '%s'", alias, existing, imp.Path)
			}
			if _, ok := mod.SymbolImports[alias]; ok {
				return nil, fmt.Errorf("module import alias '%s' conflicts with imported symbol", alias)
			}
			if child.Program.ModuleName != "" && child.Program.ModuleName != alias {
				return nil, fmt.Errorf("import %q expected module '%s', got '%s'", imp.Path, alias, child.Program.ModuleName)
			}
			mod.Imports[alias] = child
			mod.ImportPaths[alias] = imp.Path
			continue
		}
		symbols := imp.Symbols
		if imp.ObjectName != "" {
			if imp.Wildcard {
				symbols = exportedObjectMembers(child, imp.ObjectName, program.ModuleName)
			}
		} else if imp.Wildcard {
			symbols = exportedSymbols(child, program.ModuleName)
		}
		sameModule := program.ModuleName != "" && child.Program.ModuleName == program.ModuleName
		for _, symbol := range symbols {
			var (
				resolved ImportedSymbol
				ok       bool
			)
			if imp.ObjectName != "" {
				resolved, ok = resolveImportedObjectMember(child, imp.ObjectName, symbol.Name, sameModule)
			} else {
				resolved, ok = resolveImportedSymbol(child, symbol.Name, sameModule)
			}
			if !ok {
				if imp.ObjectName != "" {
					return nil, fmt.Errorf("import %q has no visible member '%s' on object '%s'", imp.Path, symbol.Name, imp.ObjectName)
				}
				return nil, fmt.Errorf("import %q has no public symbol '%s'", imp.Path, symbol.Name)
			}
			localName := symbol.Name
			if symbol.Alias != "" {
				localName = symbol.Alias
			}
			if _, ok := mod.Imports[localName]; ok {
				return nil, fmt.Errorf("imported symbol '%s' conflicts with module import alias", localName)
			}
			if existing, ok := mod.SymbolImports[localName]; ok && (existing.Module.Path != child.Path || existing.OriginalName != resolved.OriginalName) {
				return nil, fmt.Errorf("duplicate imported symbol '%s'", localName)
			}
			resolved.LocalName = localName
			mod.SymbolImports[localName] = resolved
		}
	}

	return mod, nil
}

func exportedSymbols(mod *LoadedModule, currentModule string) []parser.ImportSymbol {
	sameModule := currentModule != "" && mod.SourceProgram.ModuleName == currentModule
	out := []parser.ImportSymbol{}
	for _, fn := range mod.SourceProgram.Functions {
		if !fn.Public {
			continue
		}
		out = append(out, parser.ImportSymbol{Name: fn.Name})
	}
	for _, stmt := range mod.SourceProgram.Statements {
		valStmt, ok := stmt.(*parser.ValStmt)
		if !ok || !valStmt.Public {
			continue
		}
		for _, binding := range valStmt.Bindings {
			if binding.Name == "_" {
				continue
			}
			out = append(out, parser.ImportSymbol{Name: binding.Name})
		}
	}
	for _, decl := range mod.SourceProgram.Classes {
		if decl.Private && !sameModule {
			continue
		}
		out = append(out, parser.ImportSymbol{Name: decl.Name})
	}
	for _, decl := range mod.SourceProgram.Interfaces {
		if decl.Private && !sameModule {
			continue
		}
		out = append(out, parser.ImportSymbol{Name: decl.Name})
	}
	return out
}

func exportedObjectMembers(mod *LoadedModule, objectName string, currentModule string) []parser.ImportSymbol {
	sameModule := currentModule != "" && mod.SourceProgram.ModuleName == currentModule
	for _, decl := range mod.SourceProgram.Classes {
		if !decl.Object || decl.Name != objectName {
			continue
		}
		if decl.Private && !sameModule {
			return nil
		}
		out := []parser.ImportSymbol{}
		seen := map[string]bool{}
		for _, method := range decl.Methods {
			if method.Private && !sameModule {
				continue
			}
			if seen[method.Name] {
				continue
			}
			seen[method.Name] = true
			out = append(out, parser.ImportSymbol{Name: method.Name})
		}
		return out
	}
	return nil
}

func resolveImportedSymbol(mod *LoadedModule, name string, samePackage bool) (ImportedSymbol, bool) {
	for _, fn := range mod.SourceProgram.Functions {
		if fn.Name != name || !fn.Public {
			continue
		}
		return ImportedSymbol{OriginalName: name, Module: mod, IsFunction: true}, true
	}
	for _, stmt := range mod.SourceProgram.Statements {
		valStmt, ok := stmt.(*parser.ValStmt)
		if !ok || !valStmt.Public {
			continue
		}
		for _, binding := range valStmt.Bindings {
			if binding.Name == name {
				return ImportedSymbol{OriginalName: name, Module: mod, IsValue: true}, true
			}
		}
	}
	for _, decl := range mod.SourceProgram.Classes {
		if decl.Name != name {
			continue
		}
		if decl.Private && !samePackage {
			return ImportedSymbol{}, false
		}
		return ImportedSymbol{OriginalName: name, Module: mod}, true
	}
	for _, decl := range mod.SourceProgram.Interfaces {
		if decl.Name != name {
			continue
		}
		if decl.Private && !samePackage {
			return ImportedSymbol{}, false
		}
		return ImportedSymbol{OriginalName: name, Module: mod, IsInterface: true}, true
	}
	return ImportedSymbol{}, false
}

func resolveImportedObjectMember(mod *LoadedModule, objectName string, memberName string, samePackage bool) (ImportedSymbol, bool) {
	for _, decl := range mod.SourceProgram.Classes {
		if !decl.Object || decl.Name != objectName {
			continue
		}
		if decl.Private && !samePackage {
			return ImportedSymbol{}, false
		}
		for _, method := range decl.Methods {
			if method.Name != memberName {
				continue
			}
			if method.Private && !samePackage {
				return ImportedSymbol{}, false
			}
			return ImportedSymbol{
				OriginalName: memberName,
				ObjectName:   objectName,
				Module:       mod,
				IsFunction:   true,
			}, true
		}
		return ImportedSymbol{}, false
	}
	return ImportedSymbol{}, false
}

func findStdlibDir(start string) (string, error) {
	dir, err := filepath.Abs(start)
	if err != nil {
		return "", err
	}
	for {
		candidate := filepath.Join(dir, "stdlib")
		info, err := os.Stat(candidate)
		if err == nil && info.IsDir() {
			return candidate, nil
		}
		parent := filepath.Dir(dir)
		if parent == dir {
			return "", nil
		}
		dir = parent
	}
}

func loadPreludePrograms(stdlibDir, currentFile string) ([]*parser.Program, error) {
	entries, err := os.ReadDir(stdlibDir)
	if err != nil {
		return nil, err
	}
	var paths []string
	for _, entry := range entries {
		if entry.IsDir() || filepath.Ext(entry.Name()) != ".lum" {
			continue
		}
		path := filepath.Join(stdlibDir, entry.Name())
		if path == currentFile {
			continue
		}
		directives, err := predef.ReadDirectives(path)
		if err != nil {
			return nil, err
		}
		if directives.PreludeSkip {
			continue
		}
		paths = append(paths, path)
	}
	sort.Strings(paths)

	out := make([]*parser.Program, 0, len(paths))
	for _, path := range paths {
		src, err := os.ReadFile(path)
		if err != nil {
			return nil, err
		}
		program, err := parser.Parse(string(src))
		if err != nil {
			return nil, fmt.Errorf("parse stdlib %q: %w", filepath.Base(path), err)
		}
		out = append(out, program)
	}
	return out, nil
}

func mergePrelude(program *parser.Program, prelude []*parser.Program) *parser.Program {
	if len(prelude) == 0 {
		return program
	}
	merged := &parser.Program{
		ModuleName:  program.ModuleName,
		ModuleSpan:  program.ModuleSpan,
		Imports:     append([]parser.ImportDecl(nil), program.Imports...),
		Functions:   []*parser.FunctionDecl{},
		Interfaces:  []*parser.InterfaceDecl{},
		Classes:     []*parser.ClassDecl{},
		Statements:  append([]parser.Statement(nil), program.Statements...),
		Span:        program.Span,
	}
	for _, std := range prelude {
		merged.Functions = append(merged.Functions, std.Functions...)
		merged.Interfaces = append(merged.Interfaces, std.Interfaces...)
		merged.Classes = append(merged.Classes, std.Classes...)
	}
	merged.Functions = append(merged.Functions, program.Functions...)
	merged.Interfaces = append(merged.Interfaces, program.Interfaces...)
	merged.Classes = append(merged.Classes, program.Classes...)
	return merged
}
