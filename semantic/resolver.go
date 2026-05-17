package semantic

import (
	"a-lang/module"
	"a-lang/parser"
	"a-lang/predef"
)

type Resolver struct {
	diagnostics        []Diagnostic
	scopes             []scope
	typeScopes         []typeScope
	globals            map[string]symbol
	functions          map[string]parser.Span
	classes            map[string]parser.Span
	objects            map[string]parser.Span
	interfaces         map[string]parser.Span
	imports            map[string]importInfo
	importedGlobals    map[string]symbol
	importedClasses    map[string]parser.Span
	importedObjects    map[string]parser.Span
	importedInterfaces map[string]parser.Span
	classTypes         map[string]typeDecl
	ifaceTypes         map[string]typeDecl
	ambientValues      map[string]bool
	loopDepth          int
	currentMethodCtor  bool
}

type importInfo struct {
	functions  map[string]parser.Span
	globals    map[string]parser.Span
	classes    map[string]parser.Span
	objects    map[string]parser.Span
	interfaces map[string]parser.Span
}

type symbol struct {
	span    parser.Span
	mutable bool
}

type scope map[string]symbol
type typeScope map[string]parser.Span

type typeDecl struct {
	span  parser.Span
	arity int
}

func Analyze(program *parser.Program) []Diagnostic {
	resolver := &Resolver{
		globals:            map[string]symbol{},
		functions:          map[string]parser.Span{},
		classes:            map[string]parser.Span{},
		objects:            map[string]parser.Span{},
		interfaces:         map[string]parser.Span{},
		imports:            map[string]importInfo{},
		importedGlobals:    map[string]symbol{},
		importedClasses:    map[string]parser.Span{},
		importedObjects:    map[string]parser.Span{},
		importedInterfaces: map[string]parser.Span{},
		classTypes:         map[string]typeDecl{},
		ifaceTypes:         map[string]typeDecl{},
		ambientValues:      map[string]bool{},
	}
	resolver.installAmbientPredef()
	resolver.resolveProgram(program)
	return resolver.diagnostics
}

func AnalyzeModule(mod *module.LoadedModule) []Diagnostic {
	seen := map[string]bool{}
	var diagnostics []Diagnostic
	var walk func(*module.LoadedModule)
	walk = func(current *module.LoadedModule) {
		if seen[current.Path] {
			return
		}
		seen[current.Path] = true
		for _, imported := range current.Dependencies {
			walk(imported)
		}
		resolver := &Resolver{
			globals:            map[string]symbol{},
			functions:          map[string]parser.Span{},
			classes:            map[string]parser.Span{},
			objects:            map[string]parser.Span{},
			interfaces:         map[string]parser.Span{},
			imports:            moduleImportInfo(current),
			importedGlobals:    map[string]symbol{},
			importedClasses:    map[string]parser.Span{},
			importedObjects:    map[string]parser.Span{},
			importedInterfaces: map[string]parser.Span{},
			classTypes:         map[string]typeDecl{},
			ifaceTypes:         map[string]typeDecl{},
			ambientValues:      map[string]bool{},
		}
		resolver.installAmbientPredef()
		resolver.installDirectImports(current)
		resolver.resolveProgram(current.Program)
		diagnostics = append(diagnostics, resolver.diagnostics...)
	}
	walk(mod)
	return diagnostics
}

func (r *Resolver) installAmbientPredef() {
	registry, err := predef.Load()
	if err != nil {
		panic(err)
	}
	for name, decl := range registry.Interfaces {
		descriptor, ok := registry.Types[name]
		if !ok || !descriptor.Directives.Interpreter {
			continue
		}
		r.interfaces[name] = decl.Span
		r.ifaceTypes[name] = typeDecl{span: decl.Span, arity: len(decl.TypeParameters)}
	}
	for name, decl := range registry.Classes {
		descriptor, ok := registry.Types[name]
		if !ok || !descriptor.Directives.Interpreter {
			continue
		}
		if decl.Object {
			r.objects[name] = decl.Span
			r.ambientValues[name] = true
		} else {
			r.classes[name] = decl.Span
			r.classTypes[name] = typeDecl{span: decl.Span, arity: len(decl.TypeParameters)}
			r.ambientValues[name] = true
			if decl.Enum {
				for _, enumCase := range decl.Cases {
					r.ambientValues[enumCase.Name] = true
				}
			}
		}
	}
}

func (r *Resolver) installDirectImports(mod *module.LoadedModule) {
	for localName, symbol := range mod.SymbolImports {
		if symbol.IsFunction {
			if symbol.ObjectName != "" {
				for _, class := range symbol.Module.SourceProgram.Classes {
					if !class.Object || class.Name != symbol.ObjectName {
						continue
					}
					for _, method := range class.Methods {
						if method.Name == symbol.OriginalName {
							r.functions[localName] = method.Span
							break
						}
					}
					break
				}
			} else {
				for _, fn := range symbol.Module.SourceProgram.Functions {
					if fn.Name == symbol.OriginalName {
						r.functions[localName] = fn.Span
						break
					}
				}
			}
			continue
		}
		if symbol.IsValue {
			found := false
			for _, stmt := range symbol.Module.SourceProgram.Statements {
				valStmt, ok := stmt.(*parser.ValStmt)
				if !ok || !valStmt.Public {
					continue
				}
				for _, binding := range valStmt.Bindings {
					if binding.Name == symbol.OriginalName {
						r.importedGlobals[localName] = symbolValue(binding)
						found = true
						break
					}
				}
				if found {
					break
				}
			}
			continue
		}
		if symbol.IsInterface {
			var decl *parser.InterfaceDecl
			for _, iface := range symbol.Module.SourceProgram.Interfaces {
				if iface.Name == symbol.OriginalName {
					decl = iface
					break
				}
			}
			if decl == nil {
				continue
			}
			r.importedInterfaces[localName] = decl.Span
			r.ifaceTypes[localName] = typeDecl{span: decl.Span, arity: len(decl.TypeParameters)}
			continue
		}
		var decl *parser.ClassDecl
		for _, class := range symbol.Module.SourceProgram.Classes {
			if class.Name == symbol.OriginalName {
				decl = class
				break
			}
		}
		if decl == nil {
			continue
		}
		if decl.Object {
			r.importedObjects[localName] = decl.Span
		} else {
			r.importedClasses[localName] = decl.Span
			r.classTypes[localName] = typeDecl{span: decl.Span, arity: len(decl.TypeParameters)}
		}
	}
}

func moduleImportInfo(mod *module.LoadedModule) map[string]importInfo {
	out := map[string]importInfo{}
	currentPackage := mod.Program.PackageName
	for alias, imported := range mod.Imports {
		samePackage := currentPackage != "" && imported.Program.PackageName == currentPackage
		info := importInfo{
			functions:  map[string]parser.Span{},
			globals:    map[string]parser.Span{},
			classes:    map[string]parser.Span{},
			objects:    map[string]parser.Span{},
			interfaces: map[string]parser.Span{},
		}
		for _, fn := range imported.Program.Functions {
			if fn.Public {
				info.functions[fn.Name] = fn.Span
			}
		}
		for _, stmt := range imported.Program.Statements {
			valStmt, ok := stmt.(*parser.ValStmt)
			if !ok || !valStmt.Public {
				continue
			}
			for _, binding := range valStmt.Bindings {
				if binding.Name == "_" {
					continue
				}
				info.globals[binding.Name] = binding.Span
			}
		}
		for _, class := range imported.Program.Classes {
			if class.Private && !samePackage {
				continue
			}
			if class.Object {
				info.objects[class.Name] = class.Span
			} else {
				info.classes[class.Name] = class.Span
			}
		}
		for _, iface := range imported.Program.Interfaces {
			if iface.Private && !samePackage {
				continue
			}
			info.interfaces[iface.Name] = iface.Span
		}
		out[alias] = info
	}
	return out
}

func (r *Resolver) resolveProgram(program *parser.Program) {
	for _, fn := range program.Functions {
		if previous, exists := r.functions[fn.Name]; exists {
			r.addDiagnostic("duplicate_function", "duplicate function '"+fn.Name+"'", fn.Span)
			r.addDiagnostic("duplicate_function", "previous declaration of function '"+fn.Name+"'", previous)
			continue
		}
		r.functions[fn.Name] = fn.Span
	}
	for _, decl := range program.Interfaces {
		if previous, exists := r.interfaces[decl.Name]; exists {
			r.addDiagnostic("duplicate_interface", "duplicate interface '"+decl.Name+"'", decl.Span)
			r.addDiagnostic("duplicate_interface", "previous declaration of interface '"+decl.Name+"'", previous)
			continue
		}
		r.interfaces[decl.Name] = decl.Span
		r.ifaceTypes[decl.Name] = typeDecl{span: decl.Span, arity: len(decl.TypeParameters)}
	}
	for _, decl := range program.Classes {
		if decl.Object {
			if previous, exists := r.objects[decl.Name]; exists {
				r.addDiagnostic("duplicate_class", "duplicate object '"+decl.Name+"'", decl.Span)
				r.addDiagnostic("duplicate_class", "previous declaration of object '"+decl.Name+"'", previous)
				continue
			}
			r.objects[decl.Name] = decl.Span
			continue
		}
		if previous, exists := r.classes[decl.Name]; exists {
			r.addDiagnostic("duplicate_class", "duplicate class '"+decl.Name+"'", decl.Span)
			r.addDiagnostic("duplicate_class", "previous declaration of class '"+decl.Name+"'", previous)
			continue
		}
		r.classes[decl.Name] = decl.Span
		r.classTypes[decl.Name] = typeDecl{span: decl.Span, arity: len(decl.TypeParameters)}
	}
	r.resolveGlobals(program.Statements)

	for _, fn := range program.Functions {
		r.resolveFunction(fn)
	}
	for _, decl := range program.Interfaces {
		r.resolveInterface(decl)
	}
	for _, decl := range program.Classes {
		r.resolveClass(decl)
	}
	r.pushScope()
	for _, stmt := range program.Statements {
		if _, ok := stmt.(*parser.ValStmt); ok {
			continue
		}
		r.resolveStatement(stmt)
	}
	r.popScope()
}

func (r *Resolver) resolveGlobals(statements []parser.Statement) {
	r.pushScope()
	defer r.popScope()
	for _, stmt := range statements {
		valStmt, ok := stmt.(*parser.ValStmt)
		if !ok {
			continue
		}
		for _, value := range valStmt.Values {
			if value != nil {
				r.resolveExpr(value)
			}
		}
		for _, binding := range valStmt.Bindings {
			r.resolveTypeRef(binding.Type)
			if binding.Name == "_" {
				continue
			}
			if previous, exists := r.globals[binding.Name]; exists {
				r.addDiagnostic("duplicate_binding", "duplicate binding '"+binding.Name+"'", binding.Span)
				r.addDiagnostic("duplicate_binding", "previous declaration of '"+binding.Name+"'", previous.span)
				continue
			}
			r.globals[binding.Name] = symbol{span: binding.Span, mutable: binding.Mutable}
		}
	}
}

func (r *Resolver) resolveFunction(fn *parser.FunctionDecl) {
	r.pushTypeScope()
	defer r.popTypeScope()
	for _, param := range fn.TypeParameters {
		r.defineType(param.Name, param.Span, "duplicate_type_parameter", "duplicate type parameter '"+param.Name+"'")
	}
	r.resolveTypeParameterBounds(fn.TypeParameters)
	r.resolveTypeRef(fn.ReturnType)
	r.pushScope()
	defer r.popScope()

	for _, param := range fn.Parameters {
		r.resolveTypeRef(param.Type)
		r.defineMutableAllowShadow(param.Name, param.Span, false, "duplicate_parameter", "duplicate parameter '"+param.Name+"'", true)
	}
	r.resolveBlock(fn.Body)
}

func (r *Resolver) resolveInterface(decl *parser.InterfaceDecl) {
	r.pushTypeScope()
	defer r.popTypeScope()
	for _, param := range decl.TypeParameters {
		r.defineType(param.Name, param.Span, "duplicate_type_parameter", "duplicate type parameter '"+param.Name+"'")
	}
	r.resolveTypeParameterBounds(decl.TypeParameters)
	for _, parent := range decl.Extends {
		r.resolveTypeRef(parent)
	}
	for _, method := range decl.Methods {
		r.pushTypeScope()
		for _, param := range method.TypeParameters {
			r.defineType(param.Name, param.Span, "duplicate_type_parameter", "duplicate type parameter '"+param.Name+"'")
		}
		r.resolveTypeParameterBounds(method.TypeParameters)
		r.resolveTypeRef(method.ReturnType)
		r.pushScope()
		r.defineMutableAllowShadow("this", method.Span, false, "duplicate_binding", "duplicate binding 'this'", true)
		for _, param := range method.Parameters {
			r.resolveTypeRef(param.Type)
			r.defineMutableAllowShadow(param.Name, param.Span, false, "duplicate_parameter", "duplicate parameter '"+param.Name+"'", false)
		}
		if method.Body != nil {
			r.resolveBlockStatements(method.Body.Statements)
		}
		r.popScope()
		r.popTypeScope()
	}
}

func (r *Resolver) resolveClass(decl *parser.ClassDecl) {
	r.pushTypeScope()
	defer r.popTypeScope()
	for _, param := range decl.TypeParameters {
		r.defineType(param.Name, param.Span, "duplicate_type_parameter", "duplicate type parameter '"+param.Name+"'")
	}
	r.resolveTypeParameterBounds(decl.TypeParameters)
	for _, target := range decl.Implements {
		r.resolveTypeRef(target)
	}
	for _, field := range decl.Fields {
		if field.Type != nil {
			r.resolveTypeRef(field.Type)
		}
		if field.Initializer != nil {
			r.resolveExpr(field.Initializer)
		}
	}

	r.pushScope()
	defer r.popScope()
	r.defineMutableAllowShadow("this", decl.Span, false, "duplicate_binding", "duplicate binding 'this'", true)
	for _, field := range decl.Fields {
		r.defineMutableAllowShadow(field.Name, field.Span, field.Mutable, "duplicate_binding", "duplicate binding '"+field.Name+"'", true)
	}
	if decl.Enum {
		for _, enumCase := range decl.Cases {
			if len(enumCase.Fields) == 0 {
				r.defineMutable(enumCase.Name, enumCase.Span, false, "duplicate_binding", "duplicate binding '"+enumCase.Name+"'")
			}
		}
		for _, enumCase := range decl.Cases {
			for _, field := range enumCase.Fields {
				if field.Type != nil {
					r.resolveTypeRef(field.Type)
				}
				if field.Initializer != nil {
					r.resolveExpr(field.Initializer)
				}
			}
			for _, assignment := range enumCase.Assignments {
				r.resolveExpr(assignment.Value)
			}
			if len(enumCase.Methods) > 0 {
				r.pushScope()
				for _, field := range enumCase.Fields {
					r.defineMutableAllowShadow(field.Name, field.Span, field.Mutable, "duplicate_binding", "duplicate binding '"+field.Name+"'", true)
				}
				for _, method := range enumCase.Methods {
					r.resolveMethod(method)
				}
				r.popScope()
			}
		}
	}
	for _, method := range decl.Methods {
		r.resolveMethod(method)
	}
}

func (r *Resolver) resolveMethod(method *parser.MethodDecl) {
	r.pushTypeScope()
	defer r.popTypeScope()
	prevCtor := r.currentMethodCtor
	r.currentMethodCtor = method.Constructor
	defer func() { r.currentMethodCtor = prevCtor }()
	for _, param := range method.TypeParameters {
		r.defineType(param.Name, param.Span, "duplicate_type_parameter", "duplicate type parameter '"+param.Name+"'")
	}
	r.resolveTypeParameterBounds(method.TypeParameters)
	r.resolveTypeRef(method.ReturnType)
	r.pushScope()
	defer r.popScope()
	r.defineMutableAllowShadow("this", method.Span, false, "duplicate_binding", "duplicate binding 'this'", true)
	for _, param := range method.Parameters {
		r.resolveTypeRef(param.Type)
		r.defineMutableAllowShadow(param.Name, param.Span, false, "duplicate_parameter", "duplicate parameter '"+param.Name+"'", method.Constructor)
	}
	r.resolveBlockStatements(method.Body.Statements)
}

func (r *Resolver) resolveBlock(block *parser.BlockStmt) {
	r.pushScope()
	defer r.popScope()

	for _, stmt := range block.Statements {
		r.resolveStatement(stmt)
	}
}

func (r *Resolver) resolveStatement(stmt parser.Statement) {
	switch s := stmt.(type) {
	case *parser.ValStmt:
		for _, value := range s.Values {
			if value != nil {
				r.resolveExpr(value)
			}
		}
		for _, binding := range s.Bindings {
			r.resolveTypeRef(binding.Type)
			r.defineMutable(binding.Name, binding.Span, binding.Mutable, "duplicate_binding", "duplicate binding '"+binding.Name+"'")
		}
	case *parser.LocalFunctionStmt:
		r.defineMutable(s.Function.Name, s.Span, false, "duplicate_binding", "duplicate binding '"+s.Function.Name+"'")
		r.resolveTypeRef(s.Function.ReturnType)
		r.pushScope()
		for _, param := range s.Function.Parameters {
			r.resolveTypeRef(param.Type)
			r.defineMutable(param.Name, param.Span, false, "duplicate_parameter", "duplicate parameter '"+param.Name+"'")
		}
		r.resolveBlockStatements(s.Function.Body.Statements)
		r.popScope()
	case *parser.AssignmentStmt:
		r.resolveAssignment(s)
	case *parser.MultiAssignmentStmt:
		for _, target := range s.Targets {
			assign := &parser.AssignmentStmt{Target: target, Operator: s.Operator, Span: s.Span}
			r.resolveAssignment(assign)
		}
		for _, value := range s.Values {
			r.resolveExpr(value)
		}
	case *parser.UnwrapStmt:
		r.resolveExpr(s.Value)
		for _, binding := range s.Bindings {
			r.resolveTypeRef(binding.Type)
			if binding.Name == "_" {
				continue
			}
			r.defineMutable(binding.Name, binding.Span, false, "duplicate_binding", "duplicate binding '"+binding.Name+"'")
		}
	case *parser.UnwrapBlockStmt:
		for _, clause := range s.Clauses {
			r.resolveExpr(clause.Value)
			for _, binding := range clause.Bindings {
				r.resolveTypeRef(binding.Type)
				if binding.Name == "_" {
					continue
				}
				r.defineMutable(binding.Name, binding.Span, false, "duplicate_binding", "duplicate binding '"+binding.Name+"'")
			}
		}
	case *parser.GuardStmt:
		r.resolveExpr(s.Value)
		for _, binding := range s.Bindings {
			r.resolveTypeRef(binding.Type)
			if binding.Name == "_" {
				continue
			}
			r.defineMutable(binding.Name, binding.Span, false, "duplicate_binding", "duplicate binding '"+binding.Name+"'")
		}
		r.resolveBlock(s.Fallback)
	case *parser.GuardBlockStmt:
		r.resolveBlock(s.Fallback)
		for _, clause := range s.Clauses {
			r.resolveExpr(clause.Value)
			for _, binding := range clause.Bindings {
				r.resolveTypeRef(binding.Type)
				if binding.Name == "_" {
					continue
				}
				r.defineMutable(binding.Name, binding.Span, false, "duplicate_binding", "duplicate binding '"+binding.Name+"'")
			}
		}
	case *parser.IfStmt:
		if s.BindingValue != nil {
			r.resolveExpr(s.BindingValue)
			r.pushScope()
			for _, binding := range s.Bindings {
				if binding.Name == "_" {
					continue
				}
				r.defineMutable(binding.Name, binding.Span, false, "duplicate_binding", "duplicate binding '"+binding.Name+"'")
			}
			r.resolveBlockStatements(s.Then.Statements)
			r.popScope()
		} else {
			r.resolveExpr(s.Condition)
			r.resolveBlock(s.Then)
		}
		if s.ElseIf != nil {
			r.resolveStatement(s.ElseIf)
		}
		if s.Else != nil {
			r.resolveBlock(s.Else)
		}
	case *parser.MatchStmt:
		r.resolveExpr(s.Value)
		for _, matchCase := range s.Cases {
			r.pushScope()
			r.resolveMatchPattern(matchCase.Pattern)
			if matchCase.Guard != nil {
				r.resolveExpr(matchCase.Guard)
			}
			if matchCase.Body != nil {
				r.resolveBlockStatements(matchCase.Body.Statements)
			}
			if matchCase.Expr != nil {
				r.resolveExpr(matchCase.Expr)
			}
			r.popScope()
		}
	case *parser.WhileStmt:
		r.pushScope()
		r.loopDepth++
		r.resolveExpr(s.Condition)
		r.resolveBlockStatements(s.Body.Statements)
		r.loopDepth--
		r.popScope()
	case *parser.ForStmt:
		r.pushScope()
		for _, binding := range s.Bindings {
			if binding.Iterable != nil {
				r.resolveExpr(binding.Iterable)
			} else {
				for _, value := range binding.Values {
					if value != nil {
						r.resolveExpr(value)
					}
				}
			}
			for _, part := range binding.Bindings {
				if part.Name == "_" {
					continue
				}
				r.defineMutable(part.Name, part.Span, part.Mutable, "duplicate_binding", "duplicate binding '"+part.Name+"'")
			}
		}
		if s.Body != nil {
			r.loopDepth++
			r.resolveBlockStatements(s.Body.Statements)
			r.loopDepth--
		}
		if s.YieldBody != nil {
			r.resolveBlockStatements(s.YieldBody.Statements)
		}
		r.popScope()
	case *parser.ReturnStmt:
		r.resolveExpr(s.Value)
	case *parser.BreakStmt:
		if r.loopDepth == 0 {
			r.addDiagnostic("invalid_break", "break used outside of a loop", s.Span)
		}
	case *parser.ExprStmt:
		r.resolveExpr(s.Expr)
	}
}

func (r *Resolver) resolveBlockStatements(statements []parser.Statement) {
	for _, stmt := range statements {
		r.resolveStatement(stmt)
	}
}

func (r *Resolver) resolveExpr(expr parser.Expr) {
	switch e := expr.(type) {
	case *parser.Identifier:
		if r.isDefined(e.Name) || r.importedGlobals[e.Name].span != (parser.Span{}) || r.functions[e.Name] != (parser.Span{}) || r.classes[e.Name] != (parser.Span{}) || r.objects[e.Name] != (parser.Span{}) || r.interfaces[e.Name] != (parser.Span{}) || r.importedClasses[e.Name] != (parser.Span{}) || r.importedObjects[e.Name] != (parser.Span{}) || r.importedInterfaces[e.Name] != (parser.Span{}) || r.imports[e.Name].functions != nil || r.imports[e.Name].globals != nil || r.isBuiltinValue(e.Name) {
			return
		}
		r.addDiagnostic("undefined_name", "undefined name '"+e.Name+"'", e.Span)
	case *parser.FloatLiteral:
	case *parser.RuneLiteral:
	case *parser.BoolLiteral:
	case *parser.ListLiteral:
		for _, item := range e.Elements {
			r.resolveExpr(item)
		}
	case *parser.CallExpr:
		if ident, ok := e.Callee.(*parser.Identifier); !(ok && ident.Name == "init" && r.currentMethodCtor) {
			r.resolveExpr(e.Callee)
		}
		for _, arg := range e.Args {
			r.resolveExpr(arg.Value)
		}
	case *parser.MemberExpr:
		if ident, ok := e.Receiver.(*parser.Identifier); ok {
			if info, ok := r.imports[ident.Name]; ok {
				if info.functions[e.Name] != (parser.Span{}) || info.globals[e.Name] != (parser.Span{}) || info.classes[e.Name] != (parser.Span{}) || info.objects[e.Name] != (parser.Span{}) || info.interfaces[e.Name] != (parser.Span{}) {
					return
				}
				r.addDiagnostic("unknown_member", "unknown imported member '"+e.Name+"' on module '"+ident.Name+"'", e.Span)
				return
			}
		}
		r.resolveExpr(e.Receiver)
	case *parser.IndexExpr:
		r.resolveExpr(e.Receiver)
		r.resolveExpr(e.Index)
	case *parser.RecordUpdateExpr:
		r.resolveExpr(e.Receiver)
		for _, update := range e.Updates {
			r.resolveExpr(update.Value)
		}
	case *parser.AnonymousRecordExpr:
		seen := map[string]parser.Span{}
		for _, field := range e.Fields {
			if previous, ok := seen[field.Name]; ok {
				r.addDiagnostic("duplicate_record_field", "duplicate record field '"+field.Name+"'", field.Span)
				r.addDiagnostic("duplicate_record_field", "previous declaration of record field '"+field.Name+"'", previous)
			} else {
				seen[field.Name] = field.Span
			}
			r.resolveExpr(field.Value)
		}
	case *parser.AnonymousInterfaceExpr:
		for _, iface := range e.Interfaces {
			r.resolveTypeRef(iface)
		}
		for _, method := range e.Methods {
			r.resolveMethod(method)
		}
	case *parser.IfExpr:
		r.resolveExpr(e.Condition)
		r.pushScope()
		r.resolveBlockStatements(e.Then.Statements)
		r.popScope()
		r.pushScope()
		r.resolveBlockStatements(e.Else.Statements)
		r.popScope()
	case *parser.MatchExpr:
		r.resolveExpr(e.Value)
		for _, matchCase := range e.Cases {
			r.pushScope()
			r.resolveMatchPattern(matchCase.Pattern)
			if matchCase.Guard != nil {
				r.resolveExpr(matchCase.Guard)
			}
			if matchCase.Body != nil {
				r.resolveBlockStatements(matchCase.Body.Statements)
			}
			if matchCase.Expr != nil {
				r.resolveExpr(matchCase.Expr)
			}
			r.popScope()
		}
	case *parser.ForYieldExpr:
		r.pushScope()
		for _, binding := range e.Bindings {
			if binding.Iterable != nil {
				r.resolveExpr(binding.Iterable)
			} else {
				for _, value := range binding.Values {
					if value != nil {
						r.resolveExpr(value)
					}
				}
			}
			for _, part := range binding.Bindings {
				if part.Name == "_" {
					continue
				}
				r.defineMutable(part.Name, part.Span, part.Mutable, "duplicate_binding", "duplicate binding '"+part.Name+"'")
			}
		}
		r.resolveBlockStatements(e.YieldBody.Statements)
		r.popScope()
	case *parser.LambdaExpr:
		r.pushScope()
		for _, param := range e.Parameters {
			r.resolveTypeRef(param.Type)
			r.defineMutable(param.Name, param.Span, false, "duplicate_parameter", "duplicate parameter '"+param.Name+"'")
		}
		if e.Body != nil {
			r.resolveExpr(e.Body)
		}
		if e.BlockBody != nil {
			r.resolveBlockStatements(e.BlockBody.Statements)
		}
		r.popScope()
	case *parser.BinaryExpr:
		r.resolveExpr(e.Left)
		r.resolveExpr(e.Right)
	case *parser.IsExpr:
		r.resolveExpr(e.Left)
		r.resolveTypeRef(e.Target)
	case *parser.UnaryExpr:
		r.resolveExpr(e.Right)
	case *parser.GroupExpr:
		r.resolveExpr(e.Inner)
	}
}

func (r *Resolver) resolveAssignment(stmt *parser.AssignmentStmt) {
	switch target := stmt.Target.(type) {
	case *parser.Identifier:
		symbol, ok := r.lookup(target.Name)
		if !ok {
			if stmt.Operator == ":=" {
				r.addDiagnostic("invalid_assignment_operator", "cannot use ':=' in a declaration; use 'var "+target.Name+" = ...' for mutable bindings", target.Span)
			} else {
				r.addDiagnostic("undefined_name", "undefined name '"+target.Name+"'", target.Span)
			}
		} else if !symbol.mutable {
			r.addDiagnostic("assign_immutable", "cannot assign to immutable binding '"+target.Name+"'", target.Span)
		} else if stmt.Operator == "=" {
			r.addDiagnostic("invalid_assignment_operator", "cannot use '=' for reassignment of mutable binding '"+target.Name+"'; use ':='", target.Span)
		}
	case *parser.MemberExpr:
		r.resolveExpr(target.Receiver)
	case *parser.IndexExpr:
		r.resolveExpr(target.Receiver)
		r.resolveExpr(target.Index)
	default:
		r.addDiagnostic("invalid_assignment_target", "invalid assignment target", stmt.Span)
	}
	r.resolveExpr(stmt.Value)
}

func (r *Resolver) resolveMatchPattern(pattern parser.Pattern) {
	switch p := pattern.(type) {
	case *parser.WildcardPattern:
		return
	case *parser.BindingPattern:
		r.defineMutable(p.Name, p.Span, false, "duplicate_binding", "duplicate binding '"+p.Name+"'")
	case *parser.TypePattern:
		r.resolveTypePatternRef(p.Target)
		if p.Name != "" && p.Name != "_" {
			r.defineMutable(p.Name, p.Span, false, "duplicate_binding", "duplicate binding '"+p.Name+"'")
		}
	case *parser.LiteralPattern:
		r.resolveExpr(p.Value)
	case *parser.TuplePattern:
		for _, elem := range p.Elements {
			r.resolveMatchPattern(elem)
		}
	case *parser.ConstructorPattern:
		for _, arg := range p.Args {
			r.resolveMatchPattern(arg)
		}
	}
}

func (r *Resolver) defineMutable(name string, span parser.Span, mutable bool, code, message string) {
	r.defineMutableAllowShadow(name, span, mutable, code, message, false)
}

func (r *Resolver) defineMutableAllowShadow(name string, span parser.Span, mutable bool, code, message string, allowOuterShadow bool) {
	if name == "_" {
		return
	}
	current := r.currentScope()
	if previous, exists := current[name]; exists {
		if code == "duplicate_binding" {
			r.addDiagnostic("shadowing_binding", "binding '"+name+"' shadows an existing variable; use a different name", span)
			r.addDiagnostic("shadowing_binding", "previous declaration of '"+name+"'", previous.span)
		} else {
			r.addDiagnostic(code, message, span)
			r.addDiagnostic(code, "previous declaration of '"+name+"'", previous.span)
		}
		return
	}
	if !allowOuterShadow {
		if previous, exists := r.lookupOuter(name); exists {
			r.addDiagnostic("shadowing_binding", "binding '"+name+"' shadows an existing variable; use a different name", span)
			r.addDiagnostic("shadowing_binding", "previous declaration of '"+name+"'", previous.span)
			return
		}
	}
	current[name] = symbol{span: span, mutable: mutable}
}

func (r *Resolver) defineType(name string, span parser.Span, code, message string) {
	current := r.currentTypeScope()
	if previous, exists := current[name]; exists {
		r.addDiagnostic(code, message, span)
		r.addDiagnostic(code, "previous declaration of '"+name+"'", previous)
		return
	}
	current[name] = span
}

func (r *Resolver) resolveTypeRef(ref *parser.TypeRef) {
	if ref == nil {
		return
	}
	if ref.ReturnType != nil {
		for _, param := range ref.ParameterTypes {
			r.resolveTypeRef(param)
		}
		r.resolveTypeRef(ref.ReturnType)
		return
	}
	if len(ref.TupleElements) > 0 {
		for _, elem := range ref.TupleElements {
			r.resolveTypeRef(elem)
		}
		return
	}
	if len(ref.RecordFields) > 0 {
		seen := map[string]parser.Span{}
		for _, field := range ref.RecordFields {
			if previous, ok := seen[field.Name]; ok {
				r.addDiagnostic("duplicate_record_field", "duplicate record field '"+field.Name+"'", field.Span)
				r.addDiagnostic("duplicate_record_field", "previous declaration of record field '"+field.Name+"'", previous)
			} else {
				seen[field.Name] = field.Span
			}
			if field.Type != nil {
				r.resolveTypeRef(field.Type)
			}
		}
		return
	}
	for _, arg := range ref.Arguments {
		r.resolveTypeRef(arg)
	}
	if r.isTypeParameter(ref.Name) {
		if len(ref.Arguments) != 0 {
			r.addDiagnostic("invalid_type_arity", "type parameter '"+ref.Name+"' cannot have type arguments", ref.Span)
		}
		return
	}
	if arity, ok := builtinTypeArity(ref.Name); ok {
		if len(ref.Arguments) != arity {
			r.addDiagnostic("invalid_type_arity", "type '"+ref.Name+"' expects "+arityLabel(arity)+" type arguments", ref.Span)
		}
		return
	}
	if decl, ok := r.classTypes[ref.Name]; ok {
		if len(ref.Arguments) != decl.arity {
			r.addDiagnostic("invalid_type_arity", "type '"+ref.Name+"' expects "+arityLabel(decl.arity)+" type arguments", ref.Span)
		}
		return
	}
	if decl, ok := r.ifaceTypes[ref.Name]; ok {
		if len(ref.Arguments) != decl.arity {
			r.addDiagnostic("invalid_type_arity", "type '"+ref.Name+"' expects "+arityLabel(decl.arity)+" type arguments", ref.Span)
		}
		return
	}
	r.addDiagnostic("undefined_type", "undefined type '"+ref.Name+"'", ref.Span)
}

func (r *Resolver) resolveTypePatternRef(ref *parser.TypeRef) {
	if ref == nil {
		return
	}
	if len(ref.RecordFields) > 0 || ref.ReturnType != nil || len(ref.TupleElements) > 0 {
		r.resolveTypeRef(ref)
		return
	}
	for _, arg := range ref.Arguments {
		r.resolveTypeRef(arg)
	}
	if r.typePatternUsesErasedGeneric(ref) {
		if len(ref.Arguments) != 0 {
			r.addDiagnostic("invalid_match_pattern", "runtime type patterns cannot specify generic arguments; use the erased outer type", ref.Span)
		}
		return
	}
	r.resolveTypeRef(ref)
}

func (r *Resolver) typePatternUsesErasedGeneric(ref *parser.TypeRef) bool {
	if ref == nil || ref.Name == "" {
		return false
	}
	if arity, ok := builtinTypeArity(ref.Name); ok {
		return arity > 0
	}
	if decl, ok := r.classTypes[ref.Name]; ok {
		return decl.arity > 0
	}
	if decl, ok := r.ifaceTypes[ref.Name]; ok {
		return decl.arity > 0
	}
	return false
}

func (r *Resolver) resolveTypeParameterBounds(params []parser.TypeParameter) {
	for _, param := range params {
		for _, bound := range param.Bounds {
			r.resolveTypeRef(bound)
		}
	}
}

func (r *Resolver) isTypeParameter(name string) bool {
	for i := len(r.typeScopes) - 1; i >= 0; i-- {
		if _, ok := r.typeScopes[i][name]; ok {
			return true
		}
	}
	return false
}

func builtinTypeArity(name string) (int, bool) {
	switch name {
	case "Int", "Int64", "Bool", "Rune", "Float", "Float64", "Unit":
		return 0, true
	default:
		return 0, false
	}
}

func arityLabel(arity int) string {
	switch arity {
	case 0:
		return "0"
	case 1:
		return "1"
	case 2:
		return "2"
	default:
		return "multiple"
	}
}

func (r *Resolver) isDefined(name string) bool {
	_, ok := r.lookup(name)
	return ok
}

func (r *Resolver) lookup(name string) (symbol, bool) {
	for i := len(r.scopes) - 1; i >= 0; i-- {
		if sym, ok := r.scopes[i][name]; ok {
			return sym, true
		}
	}
	if sym, ok := r.globals[name]; ok {
		return sym, true
	}
	if sym, ok := r.importedGlobals[name]; ok {
		return sym, true
	}
	return symbol{}, false
}

func (r *Resolver) lookupOuter(name string) (symbol, bool) {
	if len(r.scopes) > 1 {
		for i := len(r.scopes) - 2; i >= 0; i-- {
			if sym, ok := r.scopes[i][name]; ok {
				return sym, true
			}
		}
	}
	if sym, ok := r.globals[name]; ok {
		return sym, true
	}
	if sym, ok := r.importedGlobals[name]; ok {
		return sym, true
	}
	return symbol{}, false
}

func symbolValue(binding parser.Binding) symbol {
	return symbol{span: binding.Span, mutable: binding.Mutable}
}

func (r *Resolver) addDiagnostic(code, message string, span parser.Span) {
	r.diagnostics = append(r.diagnostics, Diagnostic{
		Code:    code,
		Message: message,
		Span:    span,
	})
}

func (r *Resolver) pushScope() {
	r.scopes = append(r.scopes, scope{})
}

func (r *Resolver) popScope() {
	r.scopes = r.scopes[:len(r.scopes)-1]
}

func (r *Resolver) currentScope() scope {
	if len(r.scopes) == 0 {
		r.pushScope()
	}
	return r.scopes[len(r.scopes)-1]
}

func (r *Resolver) pushTypeScope() {
	r.typeScopes = append(r.typeScopes, typeScope{})
}

func (r *Resolver) popTypeScope() {
	r.typeScopes = r.typeScopes[:len(r.typeScopes)-1]
}

func (r *Resolver) currentTypeScope() typeScope {
	if len(r.typeScopes) == 0 {
		r.pushTypeScope()
	}
	return r.typeScopes[len(r.typeScopes)-1]
}

func (r *Resolver) isBuiltinValue(name string) bool {
	switch name {
	case "List", "Map", "Set", "Array":
		return true
	default:
		return r.ambientValues[name] || isImplicitOSMethod(name)
	}
}

func isImplicitOSMethod(name string) bool {
	switch name {
	case "print", "println", "printf", "panic":
		return true
	default:
		return false
	}
}
