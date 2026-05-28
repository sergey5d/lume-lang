package typecheck

import (
	"fmt"
	"strings"

	"a-lang/module"
	"a-lang/parser"
	"a-lang/predef"
	"a-lang/semantic"
)

const maxFiniteMatchDomainSize = 32

type Signature struct {
	Parameters []*Type
	ReturnType *Type
	Variadic   bool
}

type binding struct {
	typ     *Type
	mutable bool
}

type scope map[string]binding
type typeScope map[string]TypeKind

type fieldInfo struct {
	decl parser.FieldDecl
	typ  *Type
}

type methodInfo struct {
	decl *parser.MethodDecl
}

type interfaceMethodInfo struct {
	decl parser.InterfaceMethod
}

type classInfo struct {
	name         string
	decl         *parser.ClassDecl
	fields       map[string]fieldInfo
	methods      map[string][]methodInfo
	caseMethods  map[string]map[string][]methodInfo
	constructors []*parser.MethodDecl
	enumCases    map[string]parser.EnumCaseDecl
}

type interfaceInfo struct {
	decl    *parser.InterfaceDecl
	methods map[string]interfaceMethodInfo
}

type moduleInfo struct {
	functions     map[string]Signature
	functionDecls map[string]*parser.FunctionDecl
	globals       map[string]binding
	classes       map[string]classInfo
	objects       map[string]classInfo
	interfaces    map[string]interfaceInfo
}

type Result struct {
	Diagnostics []semantic.Diagnostic
	ExprTypes   map[parser.Expr]*Type
	ExprAliases map[parser.Expr]parser.Expr
}

type Checker struct {
	diagnostics            []semantic.Diagnostic
	scopes                 []scope
	typeScopes             []typeScope
	globals                map[string]binding
	functions              map[string]Signature
	functionDecls          map[string]*parser.FunctionDecl
	classes                map[string]classInfo
	objects                map[string]classInfo
	interfaces             map[string]interfaceInfo
	imports                map[string]moduleInfo
	importedGlobals        map[string]binding
	importedClasses        map[string]classInfo
	importedObjects        map[string]classInfo
	importedInterfaces     map[string]interfaceInfo
	importedInterfaceNames map[string]string
	returnTypes            []*Type
	exprTypes              map[parser.Expr]*Type
	currentClass           *parser.ClassDecl
	currentMethod          *parser.MethodDecl
	lambdaScopes           []int
	anonClassID            int
	exprAliases            map[parser.Expr]parser.Expr
}

type typeLookup interface {
	kindOf(name string) TypeKind
}

func Analyze(program *parser.Program) Result {
	c := &Checker{
		globals:                map[string]binding{},
		functions:              map[string]Signature{},
		functionDecls:          map[string]*parser.FunctionDecl{},
		classes:                map[string]classInfo{},
		objects:                map[string]classInfo{},
		interfaces:             map[string]interfaceInfo{},
		imports:                map[string]moduleInfo{},
		importedGlobals:        map[string]binding{},
		importedClasses:        map[string]classInfo{},
		importedObjects:        map[string]classInfo{},
		importedInterfaces:     map[string]interfaceInfo{},
		importedInterfaceNames: map[string]string{},
		exprTypes:              map[parser.Expr]*Type{},
		exprAliases:            map[parser.Expr]parser.Expr{},
	}
	c.installBuiltinInterfaces()
	c.collectDecls(program)
	c.checkProgram(program)
	return Result{Diagnostics: c.diagnostics, ExprTypes: c.exprTypes, ExprAliases: c.exprAliases}
}

func AnalyzeModule(mod *module.LoadedModule) Result {
	seen := map[string]Result{}
	var analyzeOne func(*module.LoadedModule) Result
	analyzeOne = func(current *module.LoadedModule) Result {
		if result, ok := seen[current.Path]; ok {
			return result
		}
		c := &Checker{
			globals:                map[string]binding{},
			functions:              map[string]Signature{},
			functionDecls:          map[string]*parser.FunctionDecl{},
			classes:                map[string]classInfo{},
			objects:                map[string]classInfo{},
			interfaces:             map[string]interfaceInfo{},
			imports:                map[string]moduleInfo{},
			importedGlobals:        map[string]binding{},
			importedClasses:        map[string]classInfo{},
			importedObjects:        map[string]classInfo{},
			importedInterfaces:     map[string]interfaceInfo{},
			importedInterfaceNames: map[string]string{},
			exprTypes:              map[parser.Expr]*Type{},
			exprAliases:            map[parser.Expr]parser.Expr{},
		}
		c.installBuiltinInterfaces()
		c.installModuleImports(current)
		c.collectDecls(current.Program)
		c.checkProgram(current.Program)
		result := Result{
			Diagnostics: append([]semantic.Diagnostic(nil), c.diagnostics...),
			ExprTypes:   c.exprTypes,
			ExprAliases: c.exprAliases,
		}
		seen[current.Path] = result
		for _, imported := range current.Dependencies {
			child := analyzeOne(imported)
			result.Diagnostics = append(result.Diagnostics, child.Diagnostics...)
		}
		seen[current.Path] = result
		return result
	}
	return analyzeOne(mod)
}

func (c *Checker) installBuiltinInterfaces() {
	registry, err := predef.Load()
	if err != nil {
		panic(err)
	}
	for _, decl := range registry.Program.Interfaces {
		if isBuiltinType(decl.Name) {
			continue
		}
		info := interfaceInfo{
			decl:    decl,
			methods: map[string]interfaceMethodInfo{},
		}
		for _, method := range decl.Methods {
			info.methods[method.Name] = interfaceMethodInfo{decl: method}
		}
		c.interfaces[decl.Name] = info
	}
	for _, decl := range registry.Program.Classes {
		if isBuiltinType(decl.Name) {
			continue
		}
		info := classInfo{
			name:        decl.Name,
			decl:        decl,
			fields:      map[string]fieldInfo{},
			methods:     map[string][]methodInfo{},
			caseMethods: map[string]map[string][]methodInfo{},
			enumCases:   map[string]parser.EnumCaseDecl{},
		}
		for _, field := range decl.Fields {
			info.fields[field.Name] = fieldInfo{decl: field}
		}
		for _, method := range decl.Methods {
			info.methods[method.Name] = append(info.methods[method.Name], methodInfo{decl: method})
			if method.Constructor {
				info.constructors = append(info.constructors, method)
			}
		}
		for _, enumCase := range decl.Cases {
			info.enumCases[enumCase.Name] = enumCase
			if len(enumCase.Methods) > 0 {
				info.caseMethods[enumCase.Name] = map[string][]methodInfo{}
				for _, method := range enumCase.Methods {
					info.caseMethods[enumCase.Name][method.Name] = append(info.caseMethods[enumCase.Name][method.Name], methodInfo{decl: method})
				}
			}
		}
		if decl.Object {
			c.objects[decl.Name] = info
		} else {
			c.classes[decl.Name] = info
		}
	}
}

func (c *Checker) installModuleImports(current *module.LoadedModule) {
	currentModule := current.Program.ModuleName
	for alias, imported := range current.Imports {
		sameModule := currentModule != "" && imported.Program.ModuleName == currentModule
		info := moduleInfo{
			functions:     map[string]Signature{},
			functionDecls: map[string]*parser.FunctionDecl{},
			globals:       map[string]binding{},
			classes:       map[string]classInfo{},
			objects:       map[string]classInfo{},
			interfaces:    map[string]interfaceInfo{},
		}
		for _, fn := range imported.Program.Functions {
			if !fn.Public {
				continue
			}
			params := make([]*Type, len(fn.Parameters))
			for i, param := range fn.Parameters {
				params[i] = fromTypeRef(param.Type, c)
			}
			info.functions[fn.Name] = Signature{
				Parameters: params,
				ReturnType: fromTypeRef(fn.ReturnType, c),
				Variadic:   len(fn.Parameters) > 0 && fn.Parameters[len(fn.Parameters)-1].Variadic,
			}
			info.functionDecls[fn.Name] = fn
		}
		for _, stmt := range imported.Program.Statements {
			valStmt, ok := stmt.(*parser.ValStmt)
			if !ok || !valStmt.Public {
				continue
			}
			for _, bindingDecl := range valStmt.Bindings {
				if bindingDecl.Name == "_" {
					continue
				}
				typ := unknownType
				if bindingDecl.Type != nil {
					typ = fromTypeRef(bindingDecl.Type, c)
				}
				info.globals[bindingDecl.Name] = binding{typ: typ, mutable: false}
			}
		}
		for _, decl := range imported.Program.Interfaces {
			if decl.Private && !sameModule {
				continue
			}
			qualified := imported.Path + "::" + decl.Name
			iface := interfaceInfo{
				decl:    decl,
				methods: map[string]interfaceMethodInfo{},
			}
			for _, method := range decl.Methods {
				iface.methods[method.Name] = interfaceMethodInfo{decl: method}
			}
			info.interfaces[decl.Name] = iface
			c.interfaces[qualified] = iface
		}
		for _, decl := range imported.Program.Classes {
			if decl.Private && !sameModule {
				continue
			}
			qualified := imported.Path + "::" + decl.Name
			class := classInfo{
				name:        qualified,
				decl:        decl,
				fields:      map[string]fieldInfo{},
				methods:     map[string][]methodInfo{},
				caseMethods: map[string]map[string][]methodInfo{},
				enumCases:   map[string]parser.EnumCaseDecl{},
			}
			for _, field := range decl.Fields {
				class.fields[field.Name] = fieldInfo{decl: field}
			}
			for _, method := range decl.Methods {
				class.methods[method.Name] = append(class.methods[method.Name], methodInfo{decl: method})
				if method.Constructor {
					class.constructors = append(class.constructors, method)
				}
			}
			for _, enumCase := range decl.Cases {
				class.enumCases[enumCase.Name] = enumCase
				if len(enumCase.Methods) > 0 {
					class.caseMethods[enumCase.Name] = map[string][]methodInfo{}
					for _, method := range enumCase.Methods {
						class.caseMethods[enumCase.Name][method.Name] = append(class.caseMethods[enumCase.Name][method.Name], methodInfo{decl: method})
					}
				}
			}
			if decl.Object {
				info.objects[decl.Name] = class
				c.objects[qualified] = class
			} else {
				info.classes[decl.Name] = class
				c.classes[qualified] = class
			}
		}
		c.imports[alias] = info
	}
	for localName, symbol := range current.SymbolImports {
		sameModule := currentModule != "" && symbol.Module.SourceProgram.ModuleName == currentModule
		if symbol.IsFunction {
			if symbol.ObjectName != "" {
				for _, decl := range symbol.Module.SourceProgram.Classes {
					if !decl.Object || decl.Name != symbol.ObjectName {
						continue
					}
					if decl.Private && !sameModule {
						break
					}
					for _, method := range decl.Methods {
						if method.Name != symbol.OriginalName {
							continue
						}
						if method.Private && !sameModule {
							break
						}
						sig := c.instantiateMethodSignature(method, decl, nil)
						c.functions[localName] = sig
						break
					}
					break
				}
				continue
			}
			for _, fn := range symbol.Module.SourceProgram.Functions {
				if fn.Name != symbol.OriginalName || !fn.Public {
					continue
				}
				params := make([]*Type, len(fn.Parameters))
				for i, param := range fn.Parameters {
					params[i] = fromTypeRef(param.Type, c)
				}
				c.functions[localName] = Signature{
					Parameters: params,
					ReturnType: fromTypeRef(fn.ReturnType, c),
					Variadic:   len(fn.Parameters) > 0 && fn.Parameters[len(fn.Parameters)-1].Variadic,
				}
				c.functionDecls[localName] = fn
				break
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
				for _, bindingDecl := range valStmt.Bindings {
					if bindingDecl.Name != symbol.OriginalName {
						continue
					}
					typ := unknownType
					if bindingDecl.Type != nil {
						typ = fromTypeRef(bindingDecl.Type, c)
					}
					c.importedGlobals[localName] = binding{typ: typ, mutable: false}
					found = true
					break
				}
				if found {
					break
				}
			}
			continue
		}
		if symbol.IsInterface {
			for _, decl := range symbol.Module.SourceProgram.Interfaces {
				if decl.Name != symbol.OriginalName {
					continue
				}
				if decl.Private && !sameModule {
					break
				}
				info := interfaceInfo{
					decl:    decl,
					methods: map[string]interfaceMethodInfo{},
				}
				for _, method := range decl.Methods {
					info.methods[method.Name] = interfaceMethodInfo{decl: method}
				}
				qualified := symbol.Module.Path + "::" + decl.Name
				c.importedInterfaces[localName] = info
				c.importedInterfaceNames[localName] = qualified
				c.interfaces[qualified] = info
				break
			}
			continue
		}
		for _, decl := range symbol.Module.SourceProgram.Classes {
			if decl.Name != symbol.OriginalName {
				continue
			}
			if decl.Private && !sameModule {
				break
			}
			class := classInfo{
				name:        symbol.Module.Path + "::" + decl.Name,
				decl:        decl,
				fields:      map[string]fieldInfo{},
				methods:     map[string][]methodInfo{},
				caseMethods: map[string]map[string][]methodInfo{},
				enumCases:   map[string]parser.EnumCaseDecl{},
			}
			for _, field := range decl.Fields {
				class.fields[field.Name] = fieldInfo{decl: field}
			}
			for _, method := range decl.Methods {
				class.methods[method.Name] = append(class.methods[method.Name], methodInfo{decl: method})
				if method.Constructor {
					class.constructors = append(class.constructors, method)
				}
			}
			for _, enumCase := range decl.Cases {
				class.enumCases[enumCase.Name] = enumCase
				if len(enumCase.Methods) > 0 {
					class.caseMethods[enumCase.Name] = map[string][]methodInfo{}
					for _, method := range enumCase.Methods {
						class.caseMethods[enumCase.Name][method.Name] = append(class.caseMethods[enumCase.Name][method.Name], methodInfo{decl: method})
					}
				}
			}
			if decl.Object {
				c.importedObjects[localName] = class
				c.objects[class.name] = class
			} else {
				c.importedClasses[localName] = class
				c.classes[class.name] = class
			}
			break
		}
	}
}

func (c *Checker) collectDecls(program *parser.Program) {
	for _, decl := range program.Interfaces {
		info := interfaceInfo{
			decl:    decl,
			methods: map[string]interfaceMethodInfo{},
		}
		for _, method := range decl.Methods {
			info.methods[method.Name] = interfaceMethodInfo{decl: method}
		}
		c.interfaces[decl.Name] = info
	}
	for _, decl := range program.Classes {
		info := classInfo{
			name:        decl.Name,
			decl:        decl,
			fields:      map[string]fieldInfo{},
			methods:     map[string][]methodInfo{},
			caseMethods: map[string]map[string][]methodInfo{},
			enumCases:   map[string]parser.EnumCaseDecl{},
		}
		for _, field := range decl.Fields {
			info.fields[field.Name] = fieldInfo{decl: field}
		}
		for _, method := range decl.Methods {
			info.methods[method.Name] = append(info.methods[method.Name], methodInfo{decl: method})
			if method.Constructor {
				info.constructors = append(info.constructors, method)
			}
		}
		for _, enumCase := range decl.Cases {
			info.enumCases[enumCase.Name] = enumCase
			if len(enumCase.Methods) > 0 {
				info.caseMethods[enumCase.Name] = map[string][]methodInfo{}
				for _, method := range enumCase.Methods {
					info.caseMethods[enumCase.Name][method.Name] = append(info.caseMethods[enumCase.Name][method.Name], methodInfo{decl: method})
				}
			}
		}
		if decl.Object {
			c.objects[decl.Name] = info
			c.globals[decl.Name] = binding{typ: &Type{Kind: TypeObject, Name: decl.Name}, mutable: false}
		} else {
			c.classes[decl.Name] = info
		}
	}
	for _, fn := range program.Functions {
		params := make([]*Type, len(fn.Parameters))
		for i, param := range fn.Parameters {
			params[i] = fromTypeRef(param.Type, c)
		}
		c.functions[fn.Name] = Signature{
			Parameters: params,
			ReturnType: fromTypeRef(fn.ReturnType, c),
			Variadic:   len(fn.Parameters) > 0 && fn.Parameters[len(fn.Parameters)-1].Variadic,
		}
		c.functionDecls[fn.Name] = fn
	}
}

func (c *Checker) checkProgram(program *parser.Program) {
	c.checkGlobals(program.Statements)
	for _, fn := range program.Functions {
		c.checkFunction(fn)
	}
	for _, decl := range program.Interfaces {
		c.checkInterface(decl)
	}
	for _, decl := range program.Classes {
		c.checkClass(decl)
	}
}

func (c *Checker) checkInterface(decl *parser.InterfaceDecl) {
	c.pushTypeScope()
	defer c.popTypeScope()
	for _, param := range decl.TypeParameters {
		c.currentTypeScope()[param.Name] = TypeParam
	}
	c.validateTypeParameterBounds(decl.TypeParameters)
	for _, method := range decl.Methods {
		if method.Name == "init" {
			c.addDiagnostic("invalid_interface_method", "interface '"+decl.Name+"': interfaces cannot declare constructors", method.Span)
		}
		c.checkInterfaceMethod(decl, method)
	}
	for _, parent := range decl.Extends {
		parentType := c.resolveDeclaredType(parent)
		if parentType.Kind != TypeInterface {
			c.addDiagnostic("invalid_interface_inheritance", "interface '"+decl.Name+"' can only inherit from interfaces", parent.Span)
		}
	}
}

func (c *Checker) checkInterfaceMethod(owner *parser.InterfaceDecl, method parser.InterfaceMethod) {
	c.pushTypeScope()
	defer c.popTypeScope()
	for _, param := range owner.TypeParameters {
		c.currentTypeScope()[param.Name] = TypeParam
	}
	for _, param := range method.TypeParameters {
		c.currentTypeScope()[param.Name] = TypeParam
	}
	c.validateTypeParameterBounds(method.TypeParameters)
	if method.Body == nil {
		return
	}
	c.pushScope()
	defer c.popScope()
	classArgs := make([]*Type, len(owner.TypeParameters))
	for i, param := range owner.TypeParameters {
		classArgs[i] = &Type{Kind: TypeParam, Name: param.Name}
	}
	c.define("this", &Type{Kind: TypeInterface, Name: owner.Name, Args: classArgs}, false)
	expectedReturn := c.resolveDeclaredType(method.ReturnType)
	c.returnTypes = append(c.returnTypes, expectedReturn)
	defer func() { c.returnTypes = c.returnTypes[:len(c.returnTypes)-1] }()
	for _, param := range method.Parameters {
		paramType := c.resolveDeclaredType(param.Type)
		if param.Variadic {
			paramType = &Type{Kind: TypeInterface, Name: "List", Args: []*Type{paramType}}
		}
		c.define(param.Name, paramType, false)
	}
	implicitReturn := c.checkBlock(method.Body)
	if method.ReturnType != nil && !isUnknown(implicitReturn) && !isUnitType(expectedReturn) {
		c.requireAssignable(implicitReturn, expectedReturn, method.Body.Span, "invalid_return_type", "cannot implicitly return "+implicitReturn.String()+" from interface method returning "+expectedReturn.String())
	}
}

func (c *Checker) checkGlobals(statements []parser.Statement) {
	c.pushScope()
	defer c.popScope()
	for _, stmt := range statements {
		switch s := stmt.(type) {
		case *parser.ValStmt:
			for i, bindingDecl := range s.Bindings {
				if bindingDecl.Deferred {
					c.addDiagnostic("invalid_deferred", "binding '"+bindingDecl.Name+"' cannot be initialized with '?' outside class fields", bindingDecl.Span)
				}
				if s.Public {
					if bindingDecl.Mutable {
						c.addDiagnostic("invalid_public_binding", "public top-level bindings must be immutable", bindingDecl.Span)
					}
					if bindingDecl.Type == nil {
						c.addDiagnostic("cannot_infer_public_binding_type", "public top-level binding '"+bindingDecl.Name+"' requires an explicit type", bindingDecl.Span)
					}
				}
				valueType := unknownType
				hasValue := i < len(s.Values) && s.Values[i] != nil
				if hasValue {
					expected := unknownType
					if bindingDecl.Type != nil {
						expected = c.resolveDeclaredType(bindingDecl.Type)
					}
					valueType = c.checkExprWithExpected(s.Values[i], expected)
				}
				declType := valueType
				if bindingDecl.Type != nil {
					declType = c.resolveDeclaredType(bindingDecl.Type)
					if hasValue {
						c.requireAssignable(valueType, declType, bindingDecl.Span, "type_mismatch", "cannot assign "+valueType.String()+" to "+declType.String())
					}
				} else if !hasValue {
					c.addDiagnostic("invalid_deferred", "binding '"+bindingDecl.Name+"' cannot be initialized with '?' outside class fields", bindingDecl.Span)
					declType = unknownType
				}
				if bindingDecl.Name != "_" {
					c.globals[bindingDecl.Name] = binding{typ: declType, mutable: bindingDecl.Mutable}
					c.define(bindingDecl.Name, declType, bindingDecl.Mutable)
				}
			}
		case *parser.ExprStmt, *parser.AssignmentStmt, *parser.MultiAssignmentStmt, *parser.IfStmt, *parser.WhileStmt, *parser.ForStmt:
			c.checkStmt(stmt)
		default:
			c.addDiagnostic("unsupported_top_level", "unsupported top-level statement for type checking", stmtSpan(stmt))
		}
	}
}

func (c *Checker) checkFunction(fn *parser.FunctionDecl) {
	c.pushTypeScope()
	defer c.popTypeScope()
	for _, param := range fn.TypeParameters {
		c.currentTypeScope()[param.Name] = TypeParam
	}
	c.validateTypeParameterBounds(fn.TypeParameters)

	c.pushScope()
	defer c.popScope()
	expectedReturn := fromTypeRef(fn.ReturnType, c)
	c.returnTypes = append(c.returnTypes, expectedReturn)
	defer func() { c.returnTypes = c.returnTypes[:len(c.returnTypes)-1] }()

	for _, param := range fn.Parameters {
		paramType := fromTypeRef(param.Type, c)
		if param.Variadic {
			paramType = &Type{Kind: TypeInterface, Name: "List", Args: []*Type{paramType}}
		}
		c.define(param.Name, paramType, false)
	}
	implicitReturn := c.checkBlock(fn.Body)
	if fn.ReturnType != nil && !isUnknown(implicitReturn) && !isUnitType(expectedReturn) {
		c.requireAssignable(implicitReturn, expectedReturn, fn.Body.Span, "invalid_return_type", "cannot implicitly return "+implicitReturn.String()+" from function returning "+expectedReturn.String())
	}
}

func (c *Checker) checkClass(decl *parser.ClassDecl) {
	info := c.classes[decl.Name]
	if decl.Object {
		info = c.objects[decl.Name]
	}

	c.pushTypeScope()
	defer c.popTypeScope()
	for _, param := range decl.TypeParameters {
		c.currentTypeScope()[param.Name] = TypeParam
	}
	c.validateTypeParameterBounds(decl.TypeParameters)

	for _, field := range decl.Fields {
		var fieldType *Type
		if field.Type != nil {
			fieldType = c.resolveDeclaredType(field.Type)
		}
		if decl.Object && !field.Mutable && field.Initializer == nil {
			c.addDiagnostic("invalid_object_field", "object '"+decl.Name+"' must initialize immutable field '"+field.Name+"'", field.Span)
		}
		if decl.Record && field.Private {
			c.addDiagnostic("invalid_record_field", "record '"+decl.Name+"' cannot declare private field '"+field.Name+"'", field.Span)
		}
		if decl.Record && field.Mutable {
			c.addDiagnostic("invalid_record_field", "record '"+decl.Name+"' cannot declare mutable field '"+field.Name+"'", field.Span)
		}
		if decl.Enum && field.Private {
			c.addDiagnostic("invalid_enum_field", "enum '"+decl.Name+"' cannot declare private field '"+field.Name+"'", field.Span)
		}
		if decl.Enum && field.Mutable {
			c.addDiagnostic("invalid_enum_field", "enum '"+decl.Name+"' cannot declare mutable field '"+field.Name+"'", field.Span)
		}
		if field.Initializer != nil {
			if field.Type != nil {
				valueType := c.checkExprWithExpected(field.Initializer, fieldType)
				c.requireAssignable(valueType, fieldType, exprSpan(field.Initializer), "type_mismatch", "cannot assign "+valueType.String()+" to "+fieldType.String())
			} else {
				fieldType = c.checkExpr(field.Initializer)
				if isUnknown(fieldType) {
					c.addDiagnostic("cannot_infer_field_type", "cannot infer type for private field '"+field.Name+"'", field.Span)
				}
			}
		} else if field.Type == nil {
			fieldType = unknownType
			c.addDiagnostic("cannot_infer_field_type", "cannot infer type for private field '"+field.Name+"' without an initializer", field.Span)
		}
		info.fields[field.Name] = fieldInfo{decl: field, typ: fieldType}
	}
	if decl.Object {
		c.objects[decl.Name] = info
	} else {
		c.classes[decl.Name] = info
	}
	if !decl.Enum && !decl.Object {
		c.checkConstructorRules(info)
	}
	for _, method := range decl.Methods {
		c.checkMethod(method, decl)
	}
	c.checkOperatorMethods(info)
	if decl.Enum {
		c.checkEnumCases(info)
	}
	for _, impl := range decl.Implements {
		if impl.Name == "Eq" {
			c.checkEqImplementation(info, impl)
			continue
		}
		c.checkInterfaceImplementation(info, impl)
	}
}

func (c *Checker) checkMethod(method *parser.MethodDecl, owner *parser.ClassDecl) {
	c.pushTypeScope()
	defer c.popTypeScope()
	for _, param := range owner.TypeParameters {
		c.currentTypeScope()[param.Name] = TypeParam
	}
	for _, param := range method.TypeParameters {
		c.currentTypeScope()[param.Name] = TypeParam
	}
	c.validateTypeParameterBounds(method.TypeParameters)

	c.pushScope()
	defer c.popScope()

	prevClass := c.currentClass
	prevMethod := c.currentMethod
	c.currentClass = owner
	c.currentMethod = method
	defer func() {
		c.currentClass = prevClass
		c.currentMethod = prevMethod
	}()

	classArgs := make([]*Type, len(owner.TypeParameters))
	for i, param := range owner.TypeParameters {
		classArgs[i] = &Type{Kind: TypeParam, Name: param.Name}
	}
	thisKind := TypeClass
	if owner.Object {
		thisKind = TypeObject
	}
	c.define("this", &Type{Kind: thisKind, Name: owner.Name, Args: classArgs}, false)

	expectedReturn := unknownType
	if !method.Constructor {
		expectedReturn = c.resolveDeclaredType(method.ReturnType)
	}
	c.returnTypes = append(c.returnTypes, expectedReturn)
	defer func() { c.returnTypes = c.returnTypes[:len(c.returnTypes)-1] }()

	for _, param := range method.Parameters {
		paramType := c.resolveDeclaredType(param.Type)
		if param.Variadic {
			paramType = &Type{Kind: TypeInterface, Name: "List", Args: []*Type{paramType}}
		}
		c.define(param.Name, paramType, false)
	}
	if owner.Enum {
		classArgs := make([]*Type, len(owner.TypeParameters))
		for i, param := range owner.TypeParameters {
			classArgs[i] = &Type{Kind: TypeParam, Name: param.Name}
		}
		for _, enumCase := range owner.Cases {
			if len(enumCase.Fields) == 0 {
				c.define(enumCase.Name, &Type{Kind: TypeClass, Name: owner.Name, Args: classArgs}, false)
			}
		}
		if method.Constructor {
			c.addDiagnostic("invalid_enum_method", "enum '"+owner.Name+"' cannot declare constructors", method.Span)
		}
	}
	if owner.Object && method.Constructor {
		c.addDiagnostic("invalid_object_method", "object '"+owner.Name+"' cannot declare constructors", method.Span)
	}
	if owner.Record && method.Constructor {
		c.addDiagnostic("invalid_record_method", "record '"+owner.Name+"': records cannot declare constructors", method.Span)
	}
	implicitReturn := c.checkBlock(method.Body)
	if !method.Constructor && method.ReturnType != nil && !isUnknown(implicitReturn) && !isUnitType(expectedReturn) {
		c.requireAssignable(implicitReturn, expectedReturn, method.Body.Span, "invalid_return_type", "cannot implicitly return "+implicitReturn.String()+" from method returning "+expectedReturn.String())
	}
}

func (c *Checker) checkInterfaceImplementation(class classInfo, impl *parser.TypeRef) {
	if impl == nil {
		return
	}
	iface, ok := c.interfaces[impl.Name]
	if !ok {
		return
	}
	subst := map[string]*Type{}
	for i, param := range iface.decl.TypeParameters {
		if i < len(impl.Arguments) {
			subst[param.Name] = c.instantiateTypeRef(impl.Arguments[i], nil)
		}
	}

	for _, method := range c.interfaceMethods(iface.decl, map[string]bool{}) {
		if method.Body != nil {
			if classMethods, ok := class.methods[method.Name]; ok && len(classMethods) > 0 {
				expected := c.instantiateInterfaceMethodSignature(method, subst)
				if classMethod, ok := c.findMatchingMethodOverload(class, method.Name, expected.Parameters); ok {
					actual := c.instantiateMethodSignature(classMethod.decl, class.decl, nil)
					c.compareSignatures(actual, expected, classMethod.decl.Span, method.Name)
				}
			}
			continue
		}
		classMethods, ok := class.methods[method.Name]
		if !ok || len(classMethods) == 0 {
			c.addDiagnostic("interface_not_implemented", "class '"+class.decl.Name+"' does not implement method '"+method.Name+"'", class.decl.Span)
			continue
		}
		expected := c.instantiateInterfaceMethodSignature(method, subst)
		classMethod, ok := c.findMatchingMethodOverload(class, method.Name, expected.Parameters)
		if !ok {
			c.addDiagnostic("interface_not_implemented", "class '"+class.decl.Name+"' does not implement method '"+method.Name+"' with matching signature", class.decl.Span)
			continue
		}
		actual := c.instantiateMethodSignature(classMethod.decl, class.decl, nil)
		c.compareSignatures(actual, expected, classMethod.decl.Span, method.Name)
	}
}

func (c *Checker) checkOperatorMethods(class classInfo) {
	for _, methods := range class.methods {
		for _, method := range methods {
			if !method.decl.Operator {
				continue
			}
			if class.decl.Object {
				c.addDiagnostic("invalid_operator_method", "objects cannot declare operators", method.decl.Span)
				continue
			}
			if !isAllowedOperatorName(method.decl.Name) {
				c.addDiagnostic("invalid_operator_method", "operator '"+method.decl.Name+"' cannot be overloaded", method.decl.Span)
				continue
			}
			switch method.decl.Name {
			case "[]", ":+", ":-", "++", "--", "+", "*", "/", "%", "|", "&", ">>", "<<", "::":
				if len(method.decl.Parameters) != 1 {
					c.addDiagnostic("invalid_operator_method", "operator '"+method.decl.Name+"' must declare exactly 1 parameter", method.decl.Span)
				}
			case "~":
				if len(method.decl.Parameters) != 0 {
					c.addDiagnostic("invalid_operator_method", "operator '~' must declare 0 parameters", method.decl.Span)
				}
			case "-":
				if len(method.decl.Parameters) != 0 && len(method.decl.Parameters) != 1 {
					c.addDiagnostic("invalid_operator_method", "operator '-' must declare 0 or 1 parameters", method.decl.Span)
				}
			}
		}
	}
}

func isAllowedOperatorName(name string) bool {
	switch name {
	case "+", "-", "*", "/", "%", "[]", ":+", ":-", "++", "--", "|", "&", ">>", "<<", "~", "::":
		return true
	default:
		return false
	}
}

func signaturesCompatible(actual Signature, expected Signature) bool {
	if actual.Variadic != expected.Variadic {
		return false
	}
	if len(actual.Parameters) != len(expected.Parameters) {
		return false
	}
	for i := range actual.Parameters {
		if !sameType(actual.Parameters[i], expected.Parameters[i]) {
			return false
		}
	}
	return sameType(actual.ReturnType, expected.ReturnType)
}

func (c *Checker) checkEnumCases(info classInfo) {
	sharedFields := map[string]parser.FieldDecl{}
	for _, field := range info.decl.Fields {
		sharedFields[field.Name] = field
	}
	seenCases := map[string]parser.Span{}
	for _, enumCase := range info.decl.Cases {
		if prev, ok := seenCases[enumCase.Name]; ok {
			c.addDiagnostic("duplicate_enum_case", "duplicate enum case '"+enumCase.Name+"'", enumCase.Span)
			c.addDiagnostic("duplicate_enum_case", "previous declaration of enum case '"+enumCase.Name+"'", prev)
			continue
		}
		seenCases[enumCase.Name] = enumCase.Span
		assigned := map[string]bool{}
		for _, field := range enumCase.Fields {
			if _, ok := sharedFields[field.Name]; ok {
				c.addDiagnostic("invalid_enum_case_field", "enum case '"+enumCase.Name+"' must assign shared field '"+field.Name+"' instead of redeclaring it", field.Span)
			}
			if field.Private {
				c.addDiagnostic("invalid_enum_case_field", "enum case '"+enumCase.Name+"' cannot declare private field '"+field.Name+"'", field.Span)
			}
			if field.Mutable {
				c.addDiagnostic("invalid_enum_case_field", "enum case '"+enumCase.Name+"' cannot declare mutable field '"+field.Name+"'", field.Span)
			}
			fieldType := c.resolveDeclaredType(field.Type)
			if field.Initializer != nil {
				valueType := c.checkExprWithExpected(field.Initializer, fieldType)
				c.requireAssignable(valueType, fieldType, exprSpan(field.Initializer), "type_mismatch", "cannot assign "+valueType.String()+" to "+fieldType.String())
			}
		}
		for _, assignment := range enumCase.Assignments {
			field, ok := sharedFields[assignment.Name]
			if !ok {
				c.addDiagnostic("unknown_member", "unknown shared enum field '"+assignment.Name+"' in case '"+enumCase.Name+"'", assignment.Span)
				c.checkExpr(assignment.Value)
				continue
			}
			if assigned[assignment.Name] {
				c.addDiagnostic("duplicate_enum_case_assignment", "duplicate assignment to shared enum field '"+assignment.Name+"' in case '"+enumCase.Name+"'", assignment.Span)
				c.checkExpr(assignment.Value)
				continue
			}
			assigned[assignment.Name] = true
			expected := c.resolveDeclaredType(field.Type)
			valueType := c.checkExprWithExpected(assignment.Value, expected)
			c.requireAssignable(valueType, expected, exprSpan(assignment.Value), "type_mismatch", "cannot assign "+valueType.String()+" to "+expected.String())
		}
		for _, field := range info.decl.Fields {
			if field.Initializer == nil && !assigned[field.Name] {
				c.addDiagnostic("invalid_enum_case", "enum case '"+enumCase.Name+"' must initialize shared field '"+field.Name+"'", enumCase.Span)
			}
		}
		for _, method := range enumCase.Methods {
			c.checkEnumCaseMethod(info.decl, enumCase, method)
		}
	}
}

func (c *Checker) checkEnumCaseMethod(owner *parser.ClassDecl, enumCase parser.EnumCaseDecl, method *parser.MethodDecl) {
	c.pushTypeScope()
	defer c.popTypeScope()
	for _, param := range owner.TypeParameters {
		c.currentTypeScope()[param.Name] = TypeParam
	}
	for _, param := range method.TypeParameters {
		c.currentTypeScope()[param.Name] = TypeParam
	}
	c.validateTypeParameterBounds(method.TypeParameters)

	c.pushScope()
	defer c.popScope()

	prevClass := c.currentClass
	prevMethod := c.currentMethod
	c.currentClass = owner
	c.currentMethod = method
	defer func() {
		c.currentClass = prevClass
		c.currentMethod = prevMethod
	}()

	classArgs := make([]*Type, len(owner.TypeParameters))
	for i, param := range owner.TypeParameters {
		classArgs[i] = &Type{Kind: TypeParam, Name: param.Name}
	}
	c.define("this", &Type{Kind: TypeClass, Name: owner.Name, Args: classArgs}, false)

	expectedReturn := c.resolveDeclaredType(method.ReturnType)
	c.returnTypes = append(c.returnTypes, expectedReturn)
	defer func() { c.returnTypes = c.returnTypes[:len(c.returnTypes)-1] }()

	for _, field := range owner.Fields {
		c.define(field.Name, c.classFieldType(owner, field), false)
	}
	for _, field := range enumCase.Fields {
		c.define(field.Name, c.resolveDeclaredType(field.Type), false)
	}
	for _, param := range method.Parameters {
		paramType := c.resolveDeclaredType(param.Type)
		if param.Variadic {
			paramType = &Type{Kind: TypeInterface, Name: "List", Args: []*Type{paramType}}
		}
		c.define(param.Name, paramType, false)
	}
	if method.Constructor {
		c.addDiagnostic("invalid_enum_method", "enum case '"+owner.Name+"."+enumCase.Name+"' cannot declare constructors", method.Span)
	}
	implicitReturn := c.checkBlock(method.Body)
	if method.ReturnType != nil && !isUnknown(implicitReturn) && !isUnitType(expectedReturn) {
		c.requireAssignable(implicitReturn, expectedReturn, method.Body.Span, "invalid_return_type", "cannot implicitly return "+implicitReturn.String()+" from method returning "+expectedReturn.String())
	}
}

func (c *Checker) interfaceMethods(decl *parser.InterfaceDecl, seen map[string]bool) []parser.InterfaceMethod {
	if decl == nil {
		return nil
	}
	key := decl.Name
	if seen[key] {
		return nil
	}
	seen[key] = true
	var methods []parser.InterfaceMethod
	added := map[string]bool{}
	for _, parent := range decl.Extends {
		info, ok := c.interfaces[parent.Name]
		if !ok {
			continue
		}
		for _, method := range c.interfaceMethods(info.decl, seen) {
			sigKey := interfaceMethodKey(method)
			if added[sigKey] {
				continue
			}
			added[sigKey] = true
			methods = append(methods, method)
		}
	}
	for _, method := range decl.Methods {
		sigKey := interfaceMethodKey(method)
		if added[sigKey] {
			continue
		}
		added[sigKey] = true
		methods = append(methods, method)
	}
	return methods
}

func interfaceMethodKey(method parser.InterfaceMethod) string {
	key := method.Name + "("
	for i, param := range method.Parameters {
		if i > 0 {
			key += ","
		}
		key += param.Type.Name
		for _, arg := range param.Type.Arguments {
			key += "[" + arg.Name + "]"
		}
	}
	key += "):"
	if method.ReturnType != nil {
		key += method.ReturnType.Name
		for _, arg := range method.ReturnType.Arguments {
			key += "[" + arg.Name + "]"
		}
	}
	return key
}

func (c *Checker) lookupInterfaceMethodInfo(decl *parser.InterfaceDecl, name string, seen map[string]bool) (interfaceMethodInfo, bool) {
	if decl == nil {
		return interfaceMethodInfo{}, false
	}
	key := decl.Name
	if seen[key] {
		return interfaceMethodInfo{}, false
	}
	seen[key] = true
	for _, method := range decl.Methods {
		if method.Name == name {
			return interfaceMethodInfo{decl: method}, true
		}
	}
	for _, parent := range decl.Extends {
		info, ok := c.interfaces[parent.Name]
		if !ok {
			continue
		}
		if method, ok := c.lookupInterfaceMethodInfo(info.decl, name, seen); ok {
			return method, true
		}
	}
	return interfaceMethodInfo{}, false
}

func (c *Checker) checkEqImplementation(class classInfo, impl *parser.TypeRef) {
	if len(impl.Arguments) != 1 {
		c.addDiagnostic("interface_not_implemented", "Eq requires exactly one type argument", impl.Span)
		return
	}
	expectedSelf := c.instantiateTypeRef(impl.Arguments[0], c.substForDecl(class.decl.TypeParameters, nil))
	classMethods, ok := class.methods["equals"]
	if !ok || len(classMethods) == 0 {
		c.addDiagnostic("interface_not_implemented", "class '"+class.decl.Name+"' does not implement method 'equals' required by Eq", class.decl.Span)
		return
	}
	method, ok := c.findMatchingMethodOverload(class, "equals", []*Type{expectedSelf})
	if !ok {
		c.addDiagnostic("interface_not_implemented", "class '"+class.decl.Name+"' does not implement method 'equals' with signature required by Eq", class.decl.Span)
		return
	}
	actual := c.instantiateMethodSignature(method.decl, class.decl, nil)
	expected := Signature{Parameters: []*Type{expectedSelf}, ReturnType: builtin("Bool")}
	c.compareSignatures(actual, expected, method.decl.Span, "equals")
}

func (c *Checker) compareSignatures(actual, expected Signature, span parser.Span, name string) {
	if len(actual.Parameters) != len(expected.Parameters) {
		c.addDiagnostic("interface_not_implemented", "method '"+name+"' has wrong parameter count", span)
		return
	}
	for i := range actual.Parameters {
		if !sameType(actual.Parameters[i], expected.Parameters[i]) {
			c.addDiagnostic("interface_not_implemented", "method '"+name+"' parameter types do not match interface", span)
			return
		}
	}
	if !sameType(actual.ReturnType, expected.ReturnType) {
		c.addDiagnostic("interface_not_implemented", "method '"+name+"' return type does not match interface", span)
	}
}

func (c *Checker) checkBlock(block *parser.BlockStmt) *Type {
	c.pushScope()
	defer c.popScope()
	if block == nil || len(block.Statements) == 0 {
		return unknownType
	}
	for i := 0; i < len(block.Statements)-1; i++ {
		c.checkStmt(block.Statements[i])
	}
	last := block.Statements[len(block.Statements)-1]
	if exprStmt, ok := last.(*parser.ExprStmt); ok {
		expected := unknownType
		if len(c.returnTypes) > 0 && c.returnTypes[len(c.returnTypes)-1] != nil {
			expected = c.returnTypes[len(c.returnTypes)-1]
		}
		return c.checkExprWithExpected(exprStmt.Expr, expected)
	}
	c.checkStmt(last)
	return unknownType
}

func (c *Checker) bindingValueTypes(bindings []parser.Binding, values []parser.Expr, span parser.Span) []*Type {
	if len(bindings) == 0 || len(values) == 0 {
		return nil
	}
	if len(bindings) == len(values) {
		out := make([]*Type, len(values))
		for i, value := range values {
			if value == nil {
				out[i] = nil
				continue
			}
			expected := unknownType
			if bindings[i].Type != nil {
				expected = c.resolveDeclaredType(bindings[i].Type)
			}
			out[i] = c.checkExprWithExpected(value, expected)
		}
		return out
	}
	if len(values) == 1 {
		valueType := c.checkExpr(values[0])
		return c.destructureValueTypes(len(bindings), valueType, span, "invalid_binding_count", "binding")
	}
	for _, value := range values {
		c.checkExpr(value)
	}
	c.addDiagnostic("invalid_binding_count", fmt.Sprintf("binding expects %d values, got %d", len(bindings), len(values)), span)
	return nil
}

func (c *Checker) assignmentValueTypes(targetCount int, values []parser.Expr, span parser.Span) []*Type {
	if targetCount == len(values) {
		out := make([]*Type, len(values))
		for i, value := range values {
			out[i] = c.checkExpr(value)
		}
		return out
	}
	if len(values) == 1 {
		valueType := c.checkExpr(values[0])
		return c.destructureValueTypes(targetCount, valueType, span, "invalid_assignment_count", "assignment")
	}
	for _, value := range values {
		c.checkExpr(value)
	}
	c.addDiagnostic("invalid_assignment_count", fmt.Sprintf("assignment expects %d values, got %d", targetCount, len(values)), span)
	return nil
}

func (c *Checker) destructurableValueTypes(valueType *Type) ([]*Type, string, bool) {
	if valueType == nil || isUnknown(valueType) {
		return nil, "", false
	}
	if valueType.Kind == TypeTuple {
		return valueType.Args, "tuple", true
	}
	if valueType.Kind != TypeClass {
		return nil, "", false
	}
	info, ok := c.classes[valueType.Name]
	if !ok || info.decl.Enum || info.decl.Object {
		return nil, "", false
	}
	for _, field := range info.decl.Fields {
		if field.Private {
			return nil, "", false
		}
	}
	subst := c.substForDecl(info.decl.TypeParameters, valueType.Args)
	out := make([]*Type, len(info.decl.Fields))
	for i, field := range info.decl.Fields {
		out[i] = c.instantiateTypeRef(field.Type, subst)
	}
	return out, "destructured", true
}

func (c *Checker) destructureValueTypes(count int, valueType *Type, span parser.Span, code string, context string) []*Type {
	parts, kind, ok := c.destructurableValueTypes(valueType)
	if !ok {
		c.addDiagnostic(code, fmt.Sprintf("%s expects %d values, got 1", context, count), span)
		return []*Type{valueType}
	}
	if len(parts) != count {
		c.addDiagnostic(code, fmt.Sprintf("%s expects %d %s values, got %d", context, count, kind, len(parts)), span)
	}
	return parts
}

func (c *Checker) exprHasEffect(expr parser.Expr) bool {
	switch e := expr.(type) {
	case *parser.CallExpr:
		return true
	case *parser.GroupExpr:
		return c.exprHasEffect(e.Inner)
	case *parser.BlockExpr:
		return c.blockHasEffect(e.Body)
	case *parser.IfExpr:
		return c.blockHasEffect(e.Then) || c.blockHasEffect(e.Else)
	case *parser.ForYieldExpr:
		return c.blockHasEffect(e.YieldBody)
	default:
		return false
	}
}

func (c *Checker) blockHasEffect(block *parser.BlockStmt) bool {
	if block == nil {
		return false
	}
	for _, stmt := range block.Statements {
		switch s := stmt.(type) {
		case *parser.ExprStmt:
			if c.exprHasEffect(s.Expr) {
				return true
			}
		default:
			return true
		}
	}
	return false
}

func (c *Checker) defineForBindingParts(bindings []parser.Binding, bindingTypes []*Type) {
	for i, part := range bindings {
		if part.Name == "_" {
			continue
		}
		bindingType := unknownType
		if i < len(bindingTypes) && bindingTypes[i] != nil {
			bindingType = bindingTypes[i]
		}
		if part.Type != nil {
			declType := c.resolveDeclaredType(part.Type)
			c.requireAssignable(bindingType, declType, part.Span, "type_mismatch", "cannot assign "+bindingType.String()+" to "+declType.String())
			bindingType = declType
		}
		c.define(part.Name, bindingType, part.Mutable)
	}
}

func (c *Checker) checkForClause(binding parser.ForBinding) {
	if binding.Iterable != nil {
		iterType := c.checkExpr(binding.Iterable)
		elemType := c.forIterableElementType(iterType, binding.Iterable)
		bindingTypes := []*Type{elemType}
		if len(binding.Bindings) > 1 {
			bindingTypes = c.destructureValueTypes(len(binding.Bindings), elemType, binding.Span, "invalid_binding_count", "for binding")
		}
		c.defineForBindingParts(binding.Bindings, bindingTypes)
		return
	}
	valueTypes := c.bindingValueTypes(binding.Bindings, binding.Values, binding.Span)
	c.defineForBindingParts(binding.Bindings, valueTypes)
}

func (c *Checker) forIterableElementType(t *Type, expr parser.Expr) *Type {
	if t != nil && t.Kind == TypeTuple {
		c.addDiagnostic("invalid_for_range", "tuple range syntax is no longer supported; use Range(start, end)", exprSpan(expr))
		return unknownType
	}
	return c.iterableElementType(t)
}

func (c *Checker) checkStmt(stmt parser.Statement) {
	switch s := stmt.(type) {
	case *parser.ValStmt:
		valueTypes := c.bindingValueTypes(s.Bindings, s.Values, s.Span)
		for i, bindingDecl := range s.Bindings {
			if bindingDecl.Deferred {
				c.addDiagnostic("invalid_deferred", "binding '"+bindingDecl.Name+"' cannot be initialized with '?' outside class fields", bindingDecl.Span)
			}
			valueType := unknownType
			hasValue := i < len(valueTypes) && valueTypes[i] != nil
			if hasValue {
				expected := unknownType
				if bindingDecl.Type != nil {
					expected = c.resolveDeclaredType(bindingDecl.Type)
				}
				valueType = valueTypes[i]
				if expected != nil && !isUnknown(expected) {
					c.requireAssignable(valueType, expected, bindingDecl.Span, "type_mismatch", "cannot assign "+valueType.String()+" to "+expected.String())
				}
			}
			declType := valueType
			if bindingDecl.Type != nil {
				declType = c.resolveDeclaredType(bindingDecl.Type)
			} else if !hasValue {
				c.addDiagnostic("invalid_deferred", "binding '"+bindingDecl.Name+"' cannot be initialized with '?' outside class fields", bindingDecl.Span)
				declType = unknownType
			}
			if bindingDecl.Name != "_" {
				c.define(bindingDecl.Name, declType, bindingDecl.Mutable)
			}
		}
	case *parser.UnwrapStmt:
		if len(c.returnTypes) == 0 {
			c.addDiagnostic("invalid_unwrap", "unwrap binding used outside callable body", s.Span)
			return
		}
		c.checkUnwrapBindings(s.Bindings, s.Value, s.Span, true, c.returnTypes[len(c.returnTypes)-1], "invalid_unwrap", "unwrap binding")
	case *parser.UnwrapBlockStmt:
		if len(c.returnTypes) == 0 {
			c.addDiagnostic("invalid_unwrap", "unwrap used outside callable body", s.Span)
			return
		}
		if len(s.Clauses) == 0 {
			c.addDiagnostic("invalid_unwrap", "unwrap block must contain at least one '<-' binding", s.Span)
		}
		for _, clause := range s.Clauses {
			c.checkUnwrapBindings(clause.Bindings, clause.Value, clause.Span, true, c.returnTypes[len(c.returnTypes)-1], "invalid_unwrap", "unwrap binding")
		}
	case *parser.GuardStmt:
		if len(c.returnTypes) == 0 {
			c.addDiagnostic("invalid_unwrap", "unwrap used outside callable body", s.Span)
			return
		}
		c.checkUnwrapBindings(s.Bindings, s.Value, s.Span, false, nil, "invalid_unwrap", "unwrap binding")
		c.checkGuardFallbackBlock(s.Fallback, c.returnTypes[len(c.returnTypes)-1])
	case *parser.GuardBlockStmt:
		if len(c.returnTypes) == 0 {
			c.addDiagnostic("invalid_unwrap", "unwrap used outside callable body", s.Span)
			return
		}
		c.checkGuardFallbackBlock(s.Fallback, c.returnTypes[len(c.returnTypes)-1])
		if len(s.Clauses) == 0 {
			c.addDiagnostic("invalid_unwrap", "unwrap block must contain at least one '<-' binding", s.Span)
		}
		for _, clause := range s.Clauses {
			c.checkUnwrapBindings(clause.Bindings, clause.Value, clause.Span, false, nil, "invalid_unwrap", "unwrap binding")
		}
	case *parser.LetElseStmt:
		if len(c.returnTypes) == 0 {
			c.addDiagnostic("invalid_let_else", "let-else used outside callable body", s.Span)
			return
		}
		c.checkGuardFallbackBlock(s.Fallback, c.returnTypes[len(c.returnTypes)-1])
		if len(s.Clauses) > 0 {
			for _, clause := range s.Clauses {
				valueType := c.checkExpr(clause.Value)
				c.checkMatchPattern(clause.Pattern, valueType)
			}
			return
		}
		valueType := c.checkExpr(s.Value)
		c.checkMatchPattern(s.Pattern, valueType)
	case *parser.LocalFunctionStmt:
		sig := Signature{Parameters: make([]*Type, len(s.Function.Parameters)), ReturnType: fromTypeRef(s.Function.ReturnType, c), Variadic: len(s.Function.Parameters) > 0 && s.Function.Parameters[len(s.Function.Parameters)-1].Variadic}
		for i, param := range s.Function.Parameters {
			sig.Parameters[i] = fromTypeRef(param.Type, c)
		}
		c.define(s.Function.Name, functionType(s.Function.Name, sig), false)
		c.pushScope()
		defer c.popScope()
		expectedReturn := fromTypeRef(s.Function.ReturnType, c)
		c.returnTypes = append(c.returnTypes, expectedReturn)
		defer func() { c.returnTypes = c.returnTypes[:len(c.returnTypes)-1] }()
		for _, param := range s.Function.Parameters {
			paramType := fromTypeRef(param.Type, c)
			if param.Variadic {
				paramType = &Type{Kind: TypeInterface, Name: "List", Args: []*Type{paramType}}
			}
			c.define(param.Name, paramType, false)
		}
		implicitReturn := c.checkBlock(s.Function.Body)
		if s.Function.ReturnType != nil && !isUnknown(implicitReturn) && !isUnitType(expectedReturn) {
			c.requireAssignable(implicitReturn, expectedReturn, s.Function.Body.Span, "invalid_return_type", "cannot implicitly return "+implicitReturn.String()+" from function returning "+expectedReturn.String())
		}
	case *parser.AssignmentStmt:
		targetType, mutable := c.checkAssignmentTarget(s.Target, s.Span)
		valueType := c.checkExpr(s.Value)
		if !mutable {
			return
		}
		if s.Operator == "=" && !c.allowEqualsAssignment(s.Target) {
			c.addDiagnostic("invalid_assignment_operator", "cannot use '=' for reassignment of a mutable variable; use ':='", s.Span)
			return
		}
		if s.Operator != "=" && s.Operator != ":=" {
			op := s.Operator[:len(s.Operator)-1]
			c.checkBinaryOperation(targetType, valueType, op, s.Span)
		}
		c.requireAssignable(valueType, targetType, s.Span, "type_mismatch", "cannot assign "+valueType.String()+" to "+targetType.String())
	case *parser.MultiAssignmentStmt:
		valueTypes := c.assignmentValueTypes(len(s.Targets), s.Values, s.Span)
		count := len(s.Targets)
		if len(valueTypes) < count {
			count = len(valueTypes)
		}
		for i := 0; i < count; i++ {
			targetType, mutable := c.checkAssignmentTarget(s.Targets[i], s.Span)
			valueType := valueTypes[i]
			if !mutable {
				continue
			}
			if s.Operator == "=" && !c.allowEqualsAssignment(s.Targets[i]) {
				c.addDiagnostic("invalid_assignment_operator", "cannot use '=' for reassignment of a mutable variable; use ':='", s.Span)
				continue
			}
			if s.Operator != "=" && s.Operator != ":=" {
				c.addDiagnostic("invalid_assignment_operator", "multi-assignment supports only '=' and ':='", s.Span)
				continue
			}
			c.requireAssignable(valueType, targetType, s.Span, "type_mismatch", "cannot assign "+valueType.String()+" to "+targetType.String())
		}
		for i := count; i < len(s.Targets); i++ {
			c.checkAssignmentTarget(s.Targets[i], s.Span)
		}
	case *parser.IfStmt:
		if len(s.PatternClauses) > 0 {
			c.pushScope()
			for _, clause := range s.PatternClauses {
				valueType := c.checkExpr(clause.Value)
				c.checkMatchPattern(clause.Pattern, valueType)
			}
			c.checkBlockStatements(s.Then.Statements, false)
			c.popScope()
		} else if s.PatternValue != nil {
			valueType := c.checkExpr(s.PatternValue)
			c.pushScope()
			c.checkMatchPattern(s.Pattern, valueType)
			c.checkBlockStatements(s.Then.Statements, false)
			c.popScope()
		} else if s.BindingValue != nil {
			optionType := c.checkExpr(s.BindingValue)
			elemType := c.optionElementType(optionType)
			if isUnknown(elemType) {
				c.addDiagnostic("invalid_condition_type", "if binding requires Option[T]", exprSpan(s.BindingValue))
				elemType = unknownType
			}
			bindingTypes := []*Type{elemType}
			if len(s.Bindings) > 1 {
				bindingTypes = c.destructureValueTypes(len(s.Bindings), elemType, s.Span, "invalid_binding_count", "if binding")
			}
			c.pushScope()
			for i, binding := range s.Bindings {
				if binding.Name == "_" {
					continue
				}
				bindingType := unknownType
				if i < len(bindingTypes) && bindingTypes[i] != nil {
					bindingType = bindingTypes[i]
				}
				if binding.Type != nil {
					declType := c.resolveDeclaredType(binding.Type)
					c.requireAssignable(bindingType, declType, binding.Span, "type_mismatch", "cannot assign "+bindingType.String()+" to "+declType.String())
					bindingType = declType
				}
				c.define(binding.Name, bindingType, false)
			}
			c.checkBlockStatements(s.Then.Statements, false)
			c.popScope()
		} else {
			condType := c.checkExpr(s.Condition)
			c.requireAssignable(condType, builtin("Bool"), exprSpan(s.Condition), "invalid_condition_type", "if condition must be Bool")
			c.checkBlockStatements(s.Then.Statements, false)
		}
		if s.ElseIf != nil {
			c.checkStmt(s.ElseIf)
		}
		if s.Else != nil {
			c.checkBlockStatements(s.Else.Statements, false)
		}
	case *parser.MatchStmt:
		valueType := c.checkExpr(s.Value)
		for _, matchCase := range s.Cases {
			c.pushScope()
			c.checkMatchPattern(matchCase.Pattern, valueType)
			if matchCase.Guard != nil {
				guardType := c.checkExpr(matchCase.Guard)
				c.requireAssignable(guardType, builtin("Bool"), exprSpan(matchCase.Guard), "invalid_condition_type", "match guard must be Bool")
			}
			if matchCase.Body != nil {
				c.checkBlockStatements(matchCase.Body.Statements, false)
			}
			if matchCase.Expr != nil {
				c.checkExpr(matchCase.Expr)
			}
			c.popScope()
		}
		c.checkMatchUnreachableCases(valueType, s.Cases)
		if !s.Partial {
			c.checkMatchExhaustiveness(valueType, s.Cases, s.Span)
		}
	case *parser.WhileStmt:
		c.pushScope()
		condType := c.checkExpr(s.Condition)
		c.requireAssignable(condType, builtin("Bool"), exprSpan(s.Condition), "invalid_condition_type", "while condition must be Bool")
		if s.Body != nil {
			c.checkBlockStatements(s.Body.Statements, false)
		}
		c.popScope()
	case *parser.ForStmt:
		c.pushScope()
		for _, binding := range s.Bindings {
			c.checkForClause(binding)
		}
		if s.Body != nil {
			c.checkBlockStatements(s.Body.Statements, false)
		}
		if s.YieldBody != nil {
			c.checkBlockStatements(s.YieldBody.Statements, true)
		}
		c.popScope()
	case *parser.ReturnStmt:
		if len(c.returnTypes) == 0 {
			c.addDiagnostic("invalid_return", "return used outside callable body", s.Span)
			return
		}
		expected := c.returnTypes[len(c.returnTypes)-1]
		if isUnitType(expected) {
			valueType := c.checkExpr(s.Value)
			c.addDiagnostic("invalid_return_type", "cannot explicitly return "+valueType.String()+" from function returning Unit", s.Span)
			return
		}
		valueType := c.checkExprWithExpected(s.Value, expected)
		if isUnknown(expected) {
			c.returnTypes[len(c.returnTypes)-1] = valueType
			return
		}
		if !isUnknown(expected) {
			c.requireAssignable(valueType, expected, s.Span, "invalid_return_type", "cannot return "+valueType.String()+" from function returning "+expected.String())
		}
	case *parser.ExprStmt:
		before := len(c.diagnostics)
		c.checkExpr(s.Expr)
		if len(c.diagnostics) == before && !c.exprHasEffect(s.Expr) {
			c.addDiagnostic("useless_expression", "expression statement has no effect", s.Span)
		}
	}
}

func (c *Checker) checkMatchPattern(pattern parser.Pattern, valueType *Type) {
	switch p := pattern.(type) {
	case *parser.WildcardPattern:
		return
	case *parser.BindingPattern:
		c.define(p.Name, valueType, false)
	case *parser.TypePattern:
		targetType := c.resolveTypePatternTarget(p.Target, p.Span)
		if !isUnknown(valueType) && !c.patternTypeCouldMatch(valueType, targetType) {
			c.addDiagnostic("invalid_match_pattern", "type pattern does not match value type", p.Span)
		}
		if p.Name != "" && p.Name != "_" {
			bindingType := targetType
			if c.sameErasedNamedType(valueType, targetType) && len(targetType.Args) == 0 && len(valueType.Args) > 0 {
				bindingType = valueType
			}
			c.define(p.Name, bindingType, false)
		}
	case *parser.LiteralPattern:
		patternType := c.checkExpr(p.Value)
		if !sameType(valueType, patternType) && !isUnknown(valueType) && !isUnknown(patternType) {
			c.addDiagnostic("invalid_match_pattern", "pattern does not match value type", p.Span)
		}
	case *parser.TuplePattern:
		partTypes := c.tuplePatternElementTypes(len(p.Elements), valueType, p.Span)
		for i, elem := range p.Elements {
			elemType := unknownType
			if i < len(partTypes) && partTypes[i] != nil {
				elemType = partTypes[i]
			}
			c.checkMatchPattern(elem, elemType)
		}
	case *parser.ConstructorPattern:
		c.checkConstructorPattern(p, valueType)
	}
}

func (c *Checker) tuplePatternElementTypes(count int, valueType *Type, span parser.Span) []*Type {
	if valueType == nil || isUnknown(valueType) || valueType.Kind != TypeTuple {
		c.addDiagnostic("invalid_match_pattern", fmt.Sprintf("match pattern expects %d tuple values, got 1", count), span)
		return []*Type{valueType}
	}
	if len(valueType.Args) != count {
		c.addDiagnostic("invalid_match_pattern", fmt.Sprintf("match pattern expects %d tuple values, got %d", count, len(valueType.Args)), span)
	}
	return valueType.Args
}

func (c *Checker) checkMatchExhaustiveness(valueType *Type, cases []parser.MatchCase, span parser.Span) {
	if c.checkFiniteMatchExhaustiveness(valueType, cases, span) {
		return
	}
	if valueType == nil || isUnknown(valueType) || valueType.Kind != TypeClass {
		return
	}
	info, ok := c.classes[valueType.Name]
	if !ok || !info.decl.Enum {
		return
	}
	if len(info.decl.Cases) == 0 {
		return
	}
	covered := map[string]bool{}
	for _, matchCase := range cases {
		if c.patternIsCatchAll(matchCase.Pattern, valueType) {
			return
		}
		if caseName, ok := c.enumCaseNameForPattern(matchCase.Pattern, valueType); ok {
			covered[caseName] = true
		}
	}
	missing := make([]string, 0, len(info.decl.Cases))
	for _, enumCase := range info.decl.Cases {
		if !covered[enumCase.Name] {
			missing = append(missing, enumCase.Name)
		}
	}
	if len(missing) == 0 {
		return
	}
	c.addDiagnostic("non_exhaustive_match", "match does not cover enum cases: "+joinNames(missing), span)
}

type matchDomainValue struct {
	kind    string
	name    string
	literal string
	args    []matchDomainValue
}

func (c *Checker) checkMatchUnreachableCases(valueType *Type, cases []parser.MatchCase) {
	domain, finite := c.enumerateMatchDomain(valueType)
	if finite {
		remaining := append([]matchDomainValue(nil), domain...)
		for _, matchCase := range cases {
			if len(remaining) == 0 {
				c.addDiagnostic("unreachable_match_case", "match case is unreachable", matchCase.Span)
				continue
			}
			covered := c.coveredDomainValues(matchCase.Pattern, valueType, domain)
			if matchCase.Guard == nil {
				fresh := intersectDomainValues(covered, remaining)
				if len(covered) > 0 && len(fresh) == 0 {
					c.addDiagnostic("unreachable_match_case", "match case is unreachable", matchCase.Span)
					continue
				}
				remaining = subtractDomainValues(remaining, covered)
			}
		}
		return
	}
	coveredAll := false
	for _, matchCase := range cases {
		if coveredAll {
			c.addDiagnostic("unreachable_match_case", "match case is unreachable", matchCase.Span)
			continue
		}
		if matchCase.Guard == nil && c.patternIsCatchAll(matchCase.Pattern, valueType) {
			coveredAll = true
		}
	}
}

func (c *Checker) checkFiniteMatchExhaustiveness(valueType *Type, cases []parser.MatchCase, span parser.Span) bool {
	domain, ok := c.enumerateMatchDomain(valueType)
	if !ok {
		return false
	}
	remaining := append([]matchDomainValue(nil), domain...)
	for _, matchCase := range cases {
		if matchCase.Guard != nil {
			continue
		}
		covered := c.coveredDomainValues(matchCase.Pattern, valueType, domain)
		remaining = subtractDomainValues(remaining, covered)
	}
	if len(remaining) == 0 {
		return true
	}
	names := make([]string, 0, len(remaining))
	seen := map[string]bool{}
	for _, value := range remaining {
		name := value.summary()
		if !seen[name] {
			seen[name] = true
			names = append(names, name)
		}
	}
	c.addDiagnostic("non_exhaustive_match", "match does not cover cases: "+joinNames(names), span)
	return true
}

func (c *Checker) coveredDomainValues(pattern parser.Pattern, valueType *Type, domain []matchDomainValue) []matchDomainValue {
	var out []matchDomainValue
	for _, value := range domain {
		if c.patternMatchesDomainValue(pattern, valueType, value) {
			out = append(out, value)
		}
	}
	return out
}

func intersectDomainValues(left, right []matchDomainValue) []matchDomainValue {
	rightSet := map[string]bool{}
	for _, value := range right {
		rightSet[value.signature()] = true
	}
	var out []matchDomainValue
	for _, value := range left {
		if rightSet[value.signature()] {
			out = append(out, value)
		}
	}
	return out
}

func subtractDomainValues(left, covered []matchDomainValue) []matchDomainValue {
	coveredSet := map[string]bool{}
	for _, value := range covered {
		coveredSet[value.signature()] = true
	}
	out := make([]matchDomainValue, 0, len(left))
	for _, value := range left {
		if !coveredSet[value.signature()] {
			out = append(out, value)
		}
	}
	return out
}

func (v matchDomainValue) signature() string {
	switch v.kind {
	case "literal":
		return "lit:" + v.literal
	case "tuple":
		parts := make([]string, len(v.args))
		for i, arg := range v.args {
			parts[i] = arg.signature()
		}
		return "tuple(" + strings.Join(parts, ",") + ")"
	case "constructor":
		parts := make([]string, len(v.args))
		for i, arg := range v.args {
			parts[i] = arg.signature()
		}
		if len(parts) == 0 {
			return "ctor:" + v.name
		}
		return "ctor:" + v.name + "(" + strings.Join(parts, ",") + ")"
	default:
		return ""
	}
}

func (v matchDomainValue) summary() string {
	switch v.kind {
	case "literal":
		return v.literal
	case "tuple":
		parts := make([]string, len(v.args))
		for i, arg := range v.args {
			parts[i] = arg.summary()
		}
		return "(" + strings.Join(parts, ", ") + ")"
	case "constructor":
		if len(v.args) == 0 {
			return v.name
		}
		parts := make([]string, len(v.args))
		for i, arg := range v.args {
			parts[i] = arg.summary()
		}
		return v.name + "(" + strings.Join(parts, ", ") + ")"
	default:
		return ""
	}
}

func (c *Checker) enumerateMatchDomain(valueType *Type) ([]matchDomainValue, bool) {
	if valueType == nil || isUnknown(valueType) {
		return nil, false
	}
	if valueType.Kind == TypeBuiltin && valueType.Name == "Bool" {
		return []matchDomainValue{
			{kind: "literal", literal: "true"},
			{kind: "literal", literal: "false"},
		}, true
	}
	if valueType.Kind == TypeTuple {
		return c.enumerateTupleMatchDomain(valueType.Args)
	}
	if valueType.Kind != TypeClass {
		return nil, false
	}
	info, ok := c.classes[valueType.Name]
	if !ok || !info.decl.Enum {
		return nil, false
	}
	subst := c.substForDecl(info.decl.TypeParameters, valueType.Args)
	var out []matchDomainValue
	for _, enumCase := range info.decl.Cases {
		fieldTypes := make([]*Type, len(enumCase.Fields))
		for i, field := range enumCase.Fields {
			fieldTypes[i] = c.instantiateTypeRef(field.Type, subst)
		}
		caseValues, ok := c.enumerateTupleLikeMatchDomain(fieldTypes)
		if !ok {
			return nil, false
		}
		if len(caseValues) == 0 {
			out = append(out, matchDomainValue{kind: "constructor", name: enumCase.Name})
			continue
		}
		for _, combo := range caseValues {
			out = append(out, matchDomainValue{kind: "constructor", name: enumCase.Name, args: combo.args})
			if len(out) > maxFiniteMatchDomainSize {
				return nil, false
			}
		}
	}
	return out, true
}

func (c *Checker) enumerateTupleMatchDomain(elements []*Type) ([]matchDomainValue, bool) {
	values, ok := c.enumerateTupleLikeMatchDomain(elements)
	if !ok {
		return nil, false
	}
	out := make([]matchDomainValue, 0, len(values))
	for _, value := range values {
		out = append(out, matchDomainValue{kind: "tuple", args: value.args})
	}
	return out, true
}

func (c *Checker) enumerateTupleLikeMatchDomain(elements []*Type) ([]matchDomainValue, bool) {
	if len(elements) == 0 {
		return []matchDomainValue{{kind: "tuple", args: nil}}, true
	}
	acc := [][]matchDomainValue{{}}
	for _, elemType := range elements {
		domain, ok := c.enumerateMatchDomain(elemType)
		if !ok {
			return nil, false
		}
		if len(domain) == 0 {
			return nil, false
		}
		if len(acc)*len(domain) > maxFiniteMatchDomainSize {
			return nil, false
		}
		var next [][]matchDomainValue
		for _, prefix := range acc {
			for _, value := range domain {
				combined := append(append([]matchDomainValue(nil), prefix...), value)
				next = append(next, combined)
			}
		}
		acc = next
	}
	out := make([]matchDomainValue, 0, len(acc))
	for _, values := range acc {
		out = append(out, matchDomainValue{kind: "tuple", args: values})
	}
	return out, true
}

func (c *Checker) patternMatchesDomainValue(pattern parser.Pattern, valueType *Type, value matchDomainValue) bool {
	switch p := pattern.(type) {
	case *parser.WildcardPattern, *parser.BindingPattern:
		return true
	case *parser.TypePattern:
		targetType := c.resolveDeclaredType(p.Target)
		return c.patternTypeCouldMatch(valueType, targetType)
	case *parser.LiteralPattern:
		return domainLiteralMatchesPattern(p.Value, value)
	case *parser.TuplePattern:
		if value.kind != "tuple" || len(value.args) != len(p.Elements) || valueType == nil || valueType.Kind != TypeTuple {
			return false
		}
		for i, elem := range p.Elements {
			if !c.patternMatchesDomainValue(elem, valueType.Args[i], value.args[i]) {
				return false
			}
		}
		return true
	case *parser.ConstructorPattern:
		if value.kind != "constructor" || valueType == nil || valueType.Kind != TypeClass {
			return false
		}
		caseName, ok := c.enumCaseNameForPattern(p, valueType)
		if ok {
			if value.name != caseName {
				return false
			}
			info := c.classes[valueType.Name]
			var enumCase *parser.EnumCaseDecl
			for i := range info.decl.Cases {
				if info.decl.Cases[i].Name == caseName {
					enumCase = &info.decl.Cases[i]
					break
				}
			}
			if enumCase == nil || len(enumCase.Fields) != len(p.Args) || len(value.args) != len(p.Args) {
				return false
			}
			subst := c.substForDecl(info.decl.TypeParameters, valueType.Args)
			for i, arg := range p.Args {
				fieldType := c.instantiateTypeRef(enumCase.Fields[i].Type, subst)
				if !c.patternMatchesDomainValue(arg, fieldType, value.args[i]) {
					return false
				}
			}
			return true
		}
		return false
	default:
		return false
	}
}

func domainLiteralMatchesPattern(expr parser.Expr, value matchDomainValue) bool {
	if value.kind != "literal" {
		return false
	}
	switch lit := expr.(type) {
	case *parser.BoolLiteral:
		if lit.Value {
			return value.literal == "true"
		}
		return value.literal == "false"
	case *parser.IntegerLiteral:
		return value.literal == lit.Value
	case *parser.FloatLiteral:
		return value.literal == lit.Value
	case *parser.RuneLiteral:
		return value.literal == lit.Value
	case *parser.StringLiteral:
		return value.literal == lit.Value
	default:
		return false
	}
}

func (c *Checker) patternIsCatchAll(pattern parser.Pattern, valueType *Type) bool {
	switch p := pattern.(type) {
	case *parser.WildcardPattern:
		return true
	case *parser.BindingPattern:
		return true
	case *parser.TypePattern:
		targetType := c.resolveTypePatternTarget(p.Target, p.Span)
		return c.sameErasedNamedType(valueType, targetType) || c.isAssignable(valueType, targetType)
	default:
		return false
	}
}

func (c *Checker) enumCaseNameForPattern(pattern parser.Pattern, valueType *Type) (string, bool) {
	constructor, ok := pattern.(*parser.ConstructorPattern)
	if !ok {
		return "", false
	}
	info, ok := c.classes[valueType.Name]
	if !ok || !info.decl.Enum {
		return "", false
	}
	switch len(constructor.Path) {
	case 1:
		if _, ok := info.enumCases[constructor.Path[0]]; ok {
			return constructor.Path[0], true
		}
	case 2:
		if constructor.Path[0] != valueType.Name {
			return "", false
		}
		if _, ok := info.enumCases[constructor.Path[1]]; ok {
			return constructor.Path[1], true
		}
	}
	return "", false
}

func (c *Checker) patternTypeCouldMatch(valueType, targetType *Type) bool {
	if isUnknown(valueType) || isUnknown(targetType) {
		return true
	}
	if sameType(valueType, targetType) {
		return true
	}
	if c.sameErasedNamedType(valueType, targetType) {
		return true
	}
	if valueType.Kind == TypeClass && targetType.Kind == TypeClass {
		if c.isAssignable(valueType, targetType) || c.isAssignable(targetType, valueType) {
			return true
		}
	}
	if valueType.Kind == TypeClass && targetType.Kind == TypeInterface {
		if c.isAssignable(valueType, targetType) {
			return true
		}
	}
	if valueType.Kind == TypeInterface && targetType.Kind == TypeClass {
		if c.isAssignable(targetType, valueType) {
			return true
		}
	}
	return false
}

func (c *Checker) sameErasedNamedType(left, right *Type) bool {
	if left == nil || right == nil {
		return false
	}
	if left.Kind != right.Kind {
		return false
	}
	switch left.Kind {
	case TypeClass, TypeInterface:
		return left.Name == right.Name
	default:
		return false
	}
}

func (c *Checker) typePatternHasErasedGenericTarget(ref *parser.TypeRef) bool {
	if ref == nil || ref.Name == "" || len(ref.Arguments) == 0 {
		return false
	}
	if params, ok := c.typeParametersForName(ref.Name); ok {
		return len(params) > 0
	}
	if arity, ok := builtinGenericTypeArity(ref.Name); ok {
		return arity > 0
	}
	return false
}

func (c *Checker) typePatternUsesErasedGeneric(ref *parser.TypeRef) bool {
	if ref == nil || ref.Name == "" || len(ref.Arguments) != 0 {
		return false
	}
	if params, ok := c.typeParametersForName(ref.Name); ok {
		return len(params) > 0
	}
	if arity, ok := builtinGenericTypeArity(ref.Name); ok {
		return arity > 0
	}
	return false
}

func (c *Checker) resolveTypePatternTarget(ref *parser.TypeRef, span parser.Span) *Type {
	if ref == nil {
		return unknownType
	}
	if c.typePatternHasErasedGenericTarget(ref) {
		c.addDiagnostic("invalid_match_pattern", "runtime type patterns cannot specify generic arguments; use the erased outer type", span)
		if erased, ok := erasedNamedPatternType(ref.Name); ok {
			return erased
		}
	}
	if c.typePatternUsesErasedGeneric(ref) {
		if erased, ok := erasedNamedPatternType(ref.Name); ok {
			return erased
		}
	}
	return c.resolveDeclaredType(ref)
}

func erasedNamedPatternType(name string) (*Type, bool) {
	switch name {
	case "List", "Iterable", "Iterator", "Set", "Map", "Option", "Result", "Either":
		return &Type{Kind: TypeInterface, Name: name}, true
	case "Array":
		return &Type{Kind: TypeBuiltin, Name: name}, true
	default:
		return &Type{Kind: TypeClass, Name: name}, true
	}
}

func builtinGenericTypeArity(name string) (int, bool) {
	switch name {
	case "List", "Iterable", "Iterator", "Set", "Option", "Array":
		return 1, true
	case "Map", "Result", "Either":
		return 2, true
	default:
		return 0, false
	}
}

func (c *Checker) checkConstructorPattern(pattern *parser.ConstructorPattern, valueType *Type) {
	if valueType == nil || isUnknown(valueType) || valueType.Kind != TypeClass {
		c.addDiagnostic("invalid_match_pattern", "constructor pattern requires class, record, or enum value", pattern.Span)
		return
	}
	info, ok := c.classes[valueType.Name]
	if !ok {
		c.addDiagnostic("invalid_match_pattern", "constructor pattern requires class, record, or enum value", pattern.Span)
		return
	}
	if info.decl.Enum {
		caseName := ""
		switch len(pattern.Path) {
		case 1:
			caseName = pattern.Path[0]
		case 2:
			if pattern.Path[0] != valueType.Name {
				c.addDiagnostic("invalid_match_pattern", "constructor pattern does not match value type", pattern.Span)
				return
			}
			caseName = pattern.Path[1]
		default:
			c.addDiagnostic("invalid_match_pattern", "unsupported constructor pattern", pattern.Span)
			return
		}
		var enumCase *parser.EnumCaseDecl
		for i := range info.decl.Cases {
			if info.decl.Cases[i].Name == caseName {
				enumCase = &info.decl.Cases[i]
				break
			}
		}
		if enumCase == nil {
			c.addDiagnostic("invalid_match_pattern", "unknown enum case '"+caseName+"'", pattern.Span)
			return
		}
		if len(pattern.Args) != len(enumCase.Fields) {
			c.addDiagnostic("invalid_match_pattern", fmt.Sprintf("enum case '%s' expects %d pattern arguments, got %d", caseName, len(enumCase.Fields), len(pattern.Args)), pattern.Span)
			return
		}
		subst := c.substForDecl(info.decl.TypeParameters, valueType.Args)
		for i, arg := range pattern.Args {
			fieldType := c.instantiateTypeRef(enumCase.Fields[i].Type, subst)
			c.checkMatchPattern(arg, fieldType)
		}
		return
	}
	if len(pattern.Path) != 1 || pattern.Path[0] != valueType.Name {
		c.addDiagnostic("invalid_match_pattern", "constructor pattern does not match value type", pattern.Span)
		return
	}
	fieldTypes, _, ok := c.destructurableValueTypes(valueType)
	if !ok {
		c.addDiagnostic("invalid_match_pattern", "constructor pattern requires destructurable class or record", pattern.Span)
		return
	}
	if len(pattern.Args) != len(fieldTypes) {
		c.addDiagnostic("invalid_match_pattern", fmt.Sprintf("pattern '%s' expects %d arguments, got %d", valueType.Name, len(fieldTypes), len(pattern.Args)), pattern.Span)
		return
	}
	for i, arg := range pattern.Args {
		c.checkMatchPattern(arg, fieldTypes[i])
	}
}

func (c *Checker) checkBlockStatements(statements []parser.Statement, allowTailExpr bool) {
	c.pushScope()
	defer c.popScope()
	for i, stmt := range statements {
		if allowTailExpr && i == len(statements)-1 {
			_ = c.checkStmtResult(stmt, "invalid_tail_expression", "block must end with a value-producing statement")
			return
		}
		c.checkStmt(stmt)
	}
}

func (c *Checker) checkBlockResult(block *parser.BlockStmt, code, message string) *Type {
	if block == nil || len(block.Statements) == 0 {
		c.addDiagnostic(code, message, blockSpan(block))
		return unknownType
	}
	c.pushScope()
	defer c.popScope()
	for i := 0; i < len(block.Statements)-1; i++ {
		c.checkStmt(block.Statements[i])
	}
	last := block.Statements[len(block.Statements)-1]
	return c.checkStmtResult(last, code, message)
}

func (c *Checker) checkGuardFallbackBlock(block *parser.BlockStmt, expected *Type) {
	if block == nil || len(block.Statements) == 0 {
		c.addDiagnostic("invalid_unwrap", "unwrap else block must return a value", blockSpan(block))
		return
	}
	for i := 0; i < len(block.Statements)-1; i++ {
		c.checkStmt(block.Statements[i])
	}
	last := block.Statements[len(block.Statements)-1]
	if ret, ok := last.(*parser.ReturnStmt); ok {
		c.checkStmt(ret)
		return
	}
	valueType := c.checkStmtResultWithExpected(last, expected, "invalid_unwrap", "unwrap else block must end with a value-producing statement")
	if !isUnknown(expected) && !isUnknown(valueType) {
		c.requireAssignable(valueType, expected, stmtSpan(last), "invalid_unwrap", "unwrap else block value must be assignable to "+expected.String())
	}
}

func (c *Checker) checkUnwrapBindings(bindings []parser.Binding, value parser.Expr, span parser.Span, requireShortCircuit bool, returnType *Type, code, label string) {
	sourceType := c.checkExpr(value)
	successType, ok := c.unwrappableSuccessType(sourceType)
	if !ok {
		c.addDiagnostic(code, label+" requires Option[T], Result[T, E], or Either[L, R]", exprSpan(value))
		successType = unknownType
	}
	if requireShortCircuit && returnType != nil && !c.shortCircuitCompatible(sourceType, returnType) {
		c.addDiagnostic(code, label+" requires function return type compatible with "+sourceType.String(), span)
	}
	bindingTypes := []*Type{successType}
	if len(bindings) > 1 {
		bindingTypes = c.destructureValueTypes(len(bindings), successType, span, "invalid_binding_count", label)
	}
	for i, bindingDecl := range bindings {
		if bindingDecl.Name == "_" {
			continue
		}
		bindingType := unknownType
		if i < len(bindingTypes) && bindingTypes[i] != nil {
			bindingType = bindingTypes[i]
		}
		if bindingDecl.Type != nil {
			declType := c.resolveDeclaredType(bindingDecl.Type)
			c.requireAssignable(bindingType, declType, bindingDecl.Span, "type_mismatch", "cannot assign "+bindingType.String()+" to "+declType.String())
			bindingType = declType
		}
		c.define(bindingDecl.Name, bindingType, false)
	}
}

func (c *Checker) checkStmtResult(stmt parser.Statement, code, message string) *Type {
	return c.checkStmtResultWithExpected(stmt, nil, code, message)
}

func (c *Checker) checkStmtResultWithExpected(stmt parser.Statement, expected *Type, code, message string) *Type {
	switch s := stmt.(type) {
	case *parser.ExprStmt:
		if expected != nil {
			return c.checkExprWithExpected(s.Expr, expected)
		}
		return c.checkExpr(s.Expr)
	case *parser.IfStmt:
		return c.checkIfStmtResult(s, code, message)
	case *parser.MatchStmt:
		return c.checkMatchStmtResult(s, code, message)
	case *parser.WhileStmt:
		c.checkStmt(stmt)
		c.addDiagnostic(code, message, stmtSpan(stmt))
		return unknownType
	case *parser.ForStmt:
		if s.YieldBody != nil {
			return c.checkForStmtResult(s, code, message)
		}
		c.checkStmt(stmt)
		c.addDiagnostic(code, message, stmtSpan(stmt))
		return unknownType
	default:
		c.checkStmt(stmt)
		c.addDiagnostic(code, message, stmtSpan(stmt))
		return unknownType
	}
}

func (c *Checker) checkIfStmtResult(s *parser.IfStmt, code, message string) *Type {
	var thenType *Type
	if len(s.PatternClauses) > 0 {
		c.pushScope()
		for _, clause := range s.PatternClauses {
			valueType := c.checkExpr(clause.Value)
			c.checkMatchPattern(clause.Pattern, valueType)
		}
		thenType = c.checkBlockResult(s.Then, code, message)
		c.popScope()
	} else if s.BindingValue != nil {
		optionType := c.checkExpr(s.BindingValue)
		elemType := c.optionElementType(optionType)
		if isUnknown(elemType) {
			c.addDiagnostic("invalid_condition_type", "if binding requires Option[T]", exprSpan(s.BindingValue))
			elemType = unknownType
		}
		bindingTypes := []*Type{elemType}
		if len(s.Bindings) > 1 {
			bindingTypes = c.destructureValueTypes(len(s.Bindings), elemType, s.Span, "invalid_binding_count", "if binding")
		}
		c.pushScope()
		for i, binding := range s.Bindings {
			if binding.Name == "_" {
				continue
			}
			bindingType := unknownType
			if i < len(bindingTypes) && bindingTypes[i] != nil {
				bindingType = bindingTypes[i]
			}
			if binding.Type != nil {
				declType := c.resolveDeclaredType(binding.Type)
				c.requireAssignable(bindingType, declType, binding.Span, "type_mismatch", "cannot assign "+bindingType.String()+" to "+declType.String())
				bindingType = declType
			}
			c.define(binding.Name, bindingType, false)
		}
		thenType = c.checkBlockResult(s.Then, code, message)
		c.popScope()
	} else {
		condType := c.checkExpr(s.Condition)
		c.requireAssignable(condType, builtin("Bool"), exprSpan(s.Condition), "invalid_condition_type", "if condition must be Bool")
		thenType = c.checkBlockResult(s.Then, code, message)
	}

	var elseType *Type
	switch {
	case s.ElseIf != nil:
		elseType = c.checkIfStmtResult(s.ElseIf, code, message)
	case s.Else != nil:
		elseType = c.checkBlockResult(s.Else, code, message)
	default:
		c.addDiagnostic(code, message, s.Span)
		return unknownType
	}
	if !sameType(thenType, elseType) {
		c.addDiagnostic("type_mismatch", "if branches must have the same type", s.Span)
		return unknownType
	}
	return thenType
}

func (c *Checker) checkMatchStmtResult(s *parser.MatchStmt, code, message string) *Type {
	valueType := c.checkExpr(s.Value)
	var resultType *Type
	for _, matchCase := range s.Cases {
		c.pushScope()
		c.checkMatchPattern(matchCase.Pattern, valueType)
		if matchCase.Guard != nil {
			guardType := c.checkExpr(matchCase.Guard)
			c.requireAssignable(guardType, builtin("Bool"), exprSpan(matchCase.Guard), "invalid_condition_type", "match guard must be Bool")
		}
		caseType := unknownType
		if matchCase.Body != nil {
			caseType = c.checkBlockResult(matchCase.Body, code, message)
		} else if matchCase.Expr != nil {
			caseType = c.checkExpr(matchCase.Expr)
		} else {
			c.addDiagnostic(code, message, matchCase.Span)
		}
		c.popScope()
		if resultType == nil {
			resultType = caseType
			continue
		}
		if !sameType(resultType, caseType) {
			c.addDiagnostic("type_mismatch", "match cases must have the same type", s.Span)
			resultType = unknownType
		}
	}
	if !s.Partial {
		c.checkMatchUnreachableCases(valueType, s.Cases)
		c.checkMatchExhaustiveness(valueType, s.Cases, s.Span)
	}
	if resultType == nil {
		c.addDiagnostic(code, message, s.Span)
		return unknownType
	}
	if s.Partial {
		return c.optionType(resultType)
	}
	return resultType
}

func (c *Checker) checkForStmtResult(s *parser.ForStmt, code, message string) *Type {
	if s.YieldBody == nil {
		c.addDiagnostic(code, message, s.Span)
		return unknownType
	}
	c.pushScope()
	for _, binding := range s.Bindings {
		c.checkForClause(binding)
	}
	yieldType := c.checkBlockResult(s.YieldBody, code, message)
	c.popScope()
	return &Type{Kind: TypeInterface, Name: "List", Args: []*Type{yieldType}}
}

func (c *Checker) checkExpr(expr parser.Expr) *Type {
	return c.checkExprWithExpected(expr, nil)
}

func (c *Checker) checkExprWithExpected(expr parser.Expr, expected *Type) *Type {
	originalExpr := expr
	if expected != nil && expected.Kind == TypeFunction && expected.Signature != nil &&
		len(expected.Signature.Parameters) == 1 && parser.HasPlaceholderExpr(expr) {
		expr = parser.WrapPlaceholderLambdaExpr(expr)
		if expr != originalExpr {
			c.exprAliases[originalExpr] = expr
		}
	}
	var result *Type
	switch e := expr.(type) {
	case *parser.Identifier:
		if binding, depth, ok := c.lookupWithDepth(e.Name); ok {
			if c.capturesMutableOuterBinding(binding, depth) {
				c.addDiagnostic("invalid_lambda_capture", "lambdas cannot capture mutable binding '"+e.Name+"'", e.Span)
			}
			result = binding.typ
			break
		}
		if fieldType, ok := c.currentFieldType(e.Name); ok {
			result = fieldType
			break
		}
		if _, ok := c.imports[e.Name]; ok {
			result = &Type{Kind: TypeModule, Name: e.Name}
			break
		}
		if imported, ok := c.importedGlobals[e.Name]; ok {
			result = imported.typ
			break
		}
		if object, ok := c.importedObjects[e.Name]; ok {
			result = &Type{Kind: TypeObject, Name: object.name}
			break
		}
		if class, ok := c.importedClasses[e.Name]; ok {
			result = &Type{Kind: TypeClass, Name: class.name}
			break
		}
		if _, ok := c.importedInterfaces[e.Name]; ok {
			result = &Type{Kind: TypeInterface, Name: c.importedInterfaceNames[e.Name]}
			break
		}
		if sig, ok := c.functions[e.Name]; ok {
			result = functionType(e.Name, sig)
			break
		}
		if isImplicitOSMethod(e.Name) {
			if sig, ok := c.implicitOSMethodSignature(e.Name); ok {
				result = functionType(e.Name, sig)
				break
			}
		}
		if _, ok := c.classes[e.Name]; ok {
			result = &Type{Kind: TypeClass, Name: e.Name}
			break
		}
		if _, ok := c.objects[e.Name]; ok {
			result = &Type{Kind: TypeObject, Name: e.Name}
			break
		}
		if isBuiltinValue(e.Name) {
			result = unknownType
			break
		}
		c.addDiagnostic("undefined_name", "undefined name '"+e.Name+"'", e.Span)
		result = unknownType
	case *parser.IntegerLiteral:
		result = builtin("Int")
	case *parser.FloatLiteral:
		result = builtin("Float")
	case *parser.RuneLiteral:
		result = builtin("Rune")
	case *parser.BoolLiteral:
		result = builtin("Bool")
	case *parser.StringLiteral:
		result = builtin("Str")
	case *parser.UnitLiteral:
		result = builtin("Unit")
	case *parser.ListLiteral:
		if len(e.Elements) == 0 {
			result = &Type{Kind: TypeInterface, Name: "List", Args: []*Type{unknownType}}
			break
		}
		elemType := c.checkExpr(e.Elements[0])
		for _, elem := range e.Elements[1:] {
			nextType := c.checkExpr(elem)
			if !sameType(elemType, nextType) {
				c.addDiagnostic("type_mismatch", "list literal elements must have the same type", exprSpan(elem))
			}
		}
		result = &Type{Kind: TypeInterface, Name: "List", Args: []*Type{elemType}}
	case *parser.TupleLiteral:
		elements := make([]*Type, len(e.Elements))
		for i, elem := range e.Elements {
			elements[i] = c.checkExpr(elem)
		}
		result = &Type{Kind: TypeTuple, Name: "Tuple", Args: elements}
	case *parser.GroupExpr:
		result = c.checkExpr(e.Inner)
	case *parser.TryExpr:
		if len(c.returnTypes) == 0 {
			c.addDiagnostic("invalid_try", "try used outside callable body", e.Span)
			result = unknownType
			break
		}
		sourceType := c.checkExpr(e.Value)
		successType, ok := c.unwrappableSuccessType(sourceType)
		if !ok {
			c.addDiagnostic("invalid_try", "try requires Option[T], Result[T, E], or Either[L, R]", e.Span)
			result = unknownType
			break
		}
		if !c.shortCircuitCompatible(sourceType, c.returnTypes[len(c.returnTypes)-1]) {
			c.addDiagnostic("invalid_try", "try requires function return type compatible with "+sourceType.String(), e.Span)
		}
		result = successType
	case *parser.BlockExpr:
		result = c.checkBlockResult(e.Body, "invalid_block_expression", "block expression must end with an expression")
	case *parser.UnaryExpr:
		right := c.checkExpr(e.Right)
		switch e.Operator {
		case "!":
			c.requireAssignable(right, builtin("Bool"), e.Span, "invalid_unary_operand", "operator ! requires Bool")
			result = builtin("Bool")
		case "-":
			if overloaded, ok := c.resolveOperatorExprType(right, "-", nil, e.Span); ok {
				result = overloaded
				break
			}
			if !isNumeric(right) {
				c.addDiagnostic("invalid_unary_operand", "operator - requires numeric operand", e.Span)
			}
			result = right
		case "~":
			if overloaded, ok := c.resolveOperatorExprType(right, "~", nil, e.Span); ok {
				result = overloaded
				break
			}
			c.addDiagnostic("invalid_unary_operand", "operator ~ requires an overloaded operand", e.Span)
			result = unknownType
		default:
			result = unknownType
		}
	case *parser.BinaryExpr:
		left := c.checkExpr(e.Left)
		right := c.checkExpr(e.Right)
		result = c.checkBinaryOperation(left, right, e.Operator, e.Span)
	case *parser.IsExpr:
		c.checkExpr(e.Left)
		c.resolveDeclaredType(e.Target)
		result = builtin("Bool")
	case *parser.CallExpr:
		result = c.checkCall(e)
	case *parser.MemberExpr:
		result = c.checkMemberExpr(e)
	case *parser.IndexExpr:
		result = c.checkIndexExpr(e)
	case *parser.RecordUpdateExpr:
		result = c.checkRecordUpdateExpr(e)
	case *parser.AnonymousRecordExpr:
		result = c.checkAnonymousRecordExpr(e, expected)
	case *parser.AnonymousInterfaceExpr:
		result = c.checkAnonymousInterfaceExpr(e)
	case *parser.IfExpr:
		condType := c.checkExpr(e.Condition)
		c.requireAssignable(condType, builtin("Bool"), exprSpan(e.Condition), "invalid_condition_type", "if condition must be Bool")
		thenType := c.checkBlockResult(e.Then, "invalid_if_expression", "if expression branches must end with an expression")
		elseType := c.checkBlockResult(e.Else, "invalid_if_expression", "if expression branches must end with an expression")
		if !sameType(thenType, elseType) {
			c.addDiagnostic("type_mismatch", "if expression branches must have the same type", e.Span)
			result = unknownType
			break
		}
		result = thenType
	case *parser.MatchExpr:
		valueType := c.checkExpr(e.Value)
		var resultType *Type
		for _, matchCase := range e.Cases {
			c.pushScope()
			c.checkMatchPattern(matchCase.Pattern, valueType)
			if matchCase.Guard != nil {
				guardType := c.checkExpr(matchCase.Guard)
				c.requireAssignable(guardType, builtin("Bool"), exprSpan(matchCase.Guard), "invalid_condition_type", "match guard must be Bool")
			}
			caseType := unknownType
			if matchCase.Body != nil {
				caseType = c.checkBlockResult(matchCase.Body, "invalid_match_expression", "match case must end with an expression")
			} else if matchCase.Expr != nil {
				caseType = c.checkExpr(matchCase.Expr)
			}
			c.popScope()
			if resultType == nil {
				resultType = caseType
				continue
			}
			if !sameType(resultType, caseType) {
				c.addDiagnostic("type_mismatch", "match expression cases must have the same type", e.Span)
				resultType = unknownType
			}
		}
		if !e.Partial {
			c.checkMatchUnreachableCases(valueType, e.Cases)
			c.checkMatchExhaustiveness(valueType, e.Cases, e.Span)
		}
		if resultType == nil {
			result = unknownType
			break
		}
		if e.Partial {
			result = c.optionType(resultType)
		} else {
			result = resultType
		}
	case *parser.ForYieldExpr:
		c.pushScope()
		for _, binding := range e.Bindings {
			c.checkForClause(binding)
		}
		yieldType := c.checkBlockResult(e.YieldBody, "invalid_yield_expression", "yield body must end with an expression")
		c.popScope()
		result = &Type{Kind: TypeInterface, Name: "List", Args: []*Type{yieldType}}
	case *parser.LambdaExpr:
		result = c.checkLambdaExpr(e, expected)
	case *parser.PlaceholderExpr:
		result = unknownType
	default:
		result = unknownType
	}
	if originalExpr != nil {
		c.exprTypes[originalExpr] = result
	}
	if expr != nil && expr != originalExpr {
		c.exprTypes[expr] = result
	}
	return result
}

func (c *Checker) checkCall(call *parser.CallExpr) *Type {
	if ident, ok := call.Callee.(*parser.Identifier); ok {
		if ident.Name == "init" && c.currentClass != nil && c.currentMethod != nil && c.currentMethod.Constructor {
			info := c.classes[c.currentClass.Name]
			classType := &Type{Kind: TypeClass, Name: c.currentClass.Name}
			if hasNamedCallArgs(call.Args) {
				if ctor, reordered, ok := c.resolveNamedConstructorOverload(info, call.Args, call.Span); ok {
					sig := c.primaryConstructorSignature(info.decl)
					if ctor != nil {
						sig = c.instantiateMethodSignature(ctor, info.decl, nil)
					}
					for i := range reordered {
						if expected, ok := paramTypeForArg(sig, i); ok {
							argType := c.checkExprWithExpected(reordered[i], expected)
							c.requireAssignable(argType, expected, exprSpan(reordered[i]), "invalid_argument_type", "cannot pass "+argType.String()+" to parameter of type "+expected.String())
						}
					}
				}
			} else {
				argTypes := c.checkArgTypes(callArgValues(call.Args))
				if ctor, ok := c.resolveConstructorOverload(info, argTypes, call.Span); ok {
					sig := c.primaryConstructorSignature(info.decl)
					if ctor != nil {
						sig = c.instantiateMethodSignature(ctor, info.decl, nil)
					}
					for i, arg := range callArgValues(call.Args) {
						if expected, ok := paramTypeForArg(sig, i); ok {
							argType := c.checkExprWithExpected(arg, expected)
							c.requireAssignable(argType, expected, exprSpan(arg), "invalid_argument_type", "cannot pass "+argType.String()+" to parameter of type "+expected.String())
						}
					}
				}
			}
			return classType
		}
		if isBuiltinValue(ident.Name) && !c.callNameShadowed(ident.Name) {
			return c.checkBuiltinConstructorCall(ident.Name, call)
		}
		if object, ok := c.objects[ident.Name]; ok {
			for _, arg := range call.Args {
				c.checkExpr(arg.Value)
			}
			c.addDiagnostic("invalid_call_target", "object '"+object.decl.Name+"' is not callable", call.Span)
			return unknownType
		}
		if class, ok := c.classes[ident.Name]; ok {
			return c.checkConstructorCall(class, call)
		}
		if _, ok := c.importedObjects[ident.Name]; ok {
			for _, arg := range call.Args {
				c.checkExpr(arg.Value)
			}
			c.addDiagnostic("invalid_call_target", "object '"+ident.Name+"' is not callable", call.Span)
			return unknownType
		}
		if class, ok := c.importedClasses[ident.Name]; ok {
			return c.checkConstructorCall(class, call)
		}
		if fnDecl, ok := c.functionDecls[ident.Name]; ok {
			orderedArgs := callArgValues(call.Args)
			if hasNamedCallArgs(call.Args) {
				reordered, ok := c.reorderCallArgs(fnDecl.Parameters, call.Args, call.Span, "function '"+ident.Name+"'")
				if !ok {
					c.checkArgTypes(callArgValues(call.Args))
					return c.instantiateFunctionSignature(fnDecl, nil).ReturnType
				}
				orderedArgs = reordered
			}
			if len(fnDecl.TypeParameters) == 0 {
				sig := c.functions[ident.Name]
				if !validArgCount(sig, len(orderedArgs)) {
					c.addDiagnostic("invalid_argument_count", fmt.Sprintf("call expects %s arguments, got %d", expectedArgCount(sig), len(orderedArgs)), call.Span)
				}
				for i, arg := range orderedArgs {
					if expected, ok := paramTypeForArg(sig, i); ok {
						argType := c.checkExprWithExpected(arg, expected)
						c.requireAssignable(argType, expected, exprSpan(arg), "invalid_argument_type", "cannot pass "+argType.String()+" to parameter of type "+expected.String())
						continue
					}
					c.checkExpr(arg)
				}
				return sig.ReturnType
			}
			sig, ok := c.resolveFunctionCallSignature(fnDecl, orderedArgs, call.Span)
			if !ok {
				return unknownType
			}
			if !validArgCount(sig, len(orderedArgs)) {
				c.addDiagnostic("invalid_argument_count", fmt.Sprintf("call expects %s arguments, got %d", expectedArgCount(sig), len(orderedArgs)), call.Span)
			}
			for i, arg := range orderedArgs {
				if expected, ok := paramTypeForArg(sig, i); ok {
					argType := c.checkExprWithExpected(arg, expected)
					c.requireAssignable(argType, expected, exprSpan(arg), "invalid_argument_type", "cannot pass "+argType.String()+" to parameter of type "+expected.String())
					continue
				}
				c.checkExpr(arg)
			}
			return sig.ReturnType
		}
		if sig, ok := c.functions[ident.Name]; ok {
			if hasNamedCallArgs(call.Args) {
				for _, arg := range call.Args {
					c.checkExpr(arg.Value)
				}
				c.addDiagnostic("invalid_named_argument", "named arguments require a direct function or method declaration", call.Span)
				return sig.ReturnType
			}
			if !validArgCount(sig, len(call.Args)) {
				c.addDiagnostic("invalid_argument_count", fmt.Sprintf("call expects %s arguments, got %d", expectedArgCount(sig), len(call.Args)), call.Span)
			}
			for i, arg := range call.Args {
				if expected, ok := paramTypeForArg(sig, i); ok {
					argType := c.checkExprWithExpected(arg.Value, expected)
					c.requireAssignable(argType, expected, exprSpan(arg.Value), "invalid_argument_type", "cannot pass "+argType.String()+" to parameter of type "+expected.String())
					continue
				}
				c.checkExpr(arg.Value)
			}
			return sig.ReturnType
		}
		if isImplicitOSMethod(ident.Name) {
			sig, ok := c.implicitOSMethodSignature(ident.Name)
			if ok {
				if hasNamedCallArgs(call.Args) {
					for _, arg := range call.Args {
						c.checkExpr(arg.Value)
					}
					c.addDiagnostic("invalid_named_argument", "named arguments require a direct function or method declaration", call.Span)
					return sig.ReturnType
				}
				if !validArgCount(sig, len(call.Args)) {
					c.addDiagnostic("invalid_argument_count", fmt.Sprintf("call expects %s arguments, got %d", expectedArgCount(sig), len(call.Args)), call.Span)
				}
				for i, arg := range call.Args {
					if expected, ok := paramTypeForArg(sig, i); ok {
						argType := c.checkExprWithExpected(arg.Value, expected)
						c.requireAssignable(argType, expected, exprSpan(arg.Value), "invalid_argument_type", "cannot pass "+argType.String()+" to parameter of type "+expected.String())
						continue
					}
					c.checkExpr(arg.Value)
				}
				return sig.ReturnType
			}
		}
	}
	if member, ok := call.Callee.(*parser.MemberExpr); ok {
		return c.checkMethodCall(member, call.Args)
	}

	calleeType := c.checkExpr(call.Callee)
	if hasNamedCallArgs(call.Args) {
		for _, arg := range call.Args {
			c.checkExpr(arg.Value)
		}
		c.addDiagnostic("invalid_named_argument", "named arguments require a direct function, method, or constructor", call.Span)
		return unknownType
	}
	if calleeType.Kind != TypeFunction || calleeType.Signature == nil {
		for _, arg := range call.Args {
			c.checkExpr(arg.Value)
		}
		return unknownType
	}

	sig := *calleeType.Signature
	if !validArgCount(sig, len(call.Args)) {
		c.addDiagnostic("invalid_argument_count", fmt.Sprintf("call expects %s arguments, got %d", expectedArgCount(sig), len(call.Args)), call.Span)
	}
	for i, arg := range call.Args {
		var argType *Type
		if expected, ok := paramTypeForArg(sig, i); ok {
			argType = c.checkExprWithExpected(arg.Value, expected)
			c.requireAssignable(argType, expected, exprSpan(arg.Value), "invalid_argument_type", "cannot pass "+argType.String()+" to parameter of type "+expected.String())
			continue
		}
		argType = c.checkExpr(arg.Value)
	}
	return sig.ReturnType
}

func (c *Checker) resolveFunctionCallSignature(fn *parser.FunctionDecl, args []parser.Expr, span parser.Span) (Signature, bool) {
	sig := c.instantiateFunctionSignature(fn, nil)
	argCount := len(args)
	if len(fn.TypeParameters) == 0 {
		argTypes := c.checkArgTypes(args)
		if !signatureMatches(sig, argTypes) {
			c.addDiagnostic("no_matching_overload", fmt.Sprintf("function '%s' does not match %d arguments", fn.Name, len(argTypes)), span)
			return Signature{}, false
		}
		return sig, true
	}
	inferred, ok := c.inferCallableTypeArgsFromExprs(fn.TypeParameters, fn.Parameters, args, nil)
	if !ok {
		c.addDiagnostic("cannot_infer_type_args", "cannot infer type arguments for function '"+fn.Name+"'", span)
		return Signature{}, false
	}
	if !c.checkTypeArgBounds(c.typeArgsInOrder(fn.TypeParameters, inferred), fn.TypeParameters, span) {
		return Signature{}, false
	}
	sig = c.instantiateFunctionSignature(fn, inferred)
	argTypes := c.checkArgTypesWithSignature(args, sig)
	if !signatureMatches(sig, argTypes) {
		c.addDiagnostic("no_matching_overload", fmt.Sprintf("function '%s' does not match %d arguments", fn.Name, argCount), span)
		return Signature{}, false
	}
	return sig, true
}

func (c *Checker) resolveMethodCallSignature(class classInfo, receiver *Type, method *parser.MethodDecl, args []parser.Expr, span parser.Span) (Signature, bool) {
	baseSubst := c.substForDecl(class.decl.TypeParameters, receiver.Args)
	sig := c.instantiateMethodSignature(method, class.decl, baseSubst)
	if len(method.TypeParameters) == 0 {
		argTypes := c.checkArgTypes(args)
		if !signatureMatches(sig, argTypes) {
			c.addDiagnostic("no_matching_overload", fmt.Sprintf("no overload of method '%s' matches %d arguments", method.Name, len(argTypes)), span)
			return Signature{}, false
		}
		return sig, true
	}
	inferred, ok := c.inferCallableTypeArgsFromExprs(method.TypeParameters, method.Parameters, args, baseSubst)
	if !ok {
		c.addDiagnostic("cannot_infer_type_args", "cannot infer type arguments for method '"+method.Name+"'", span)
		return Signature{}, false
	}
	if !c.checkTypeArgBounds(c.typeArgsInOrder(method.TypeParameters, inferred), method.TypeParameters, span) {
		return Signature{}, false
	}
	sig = c.instantiateMethodSignature(method, class.decl, mergeSubst(inferred, baseSubst))
	argTypes := c.checkArgTypesWithSignature(args, sig)
	if !signatureMatches(sig, argTypes) {
		c.addDiagnostic("no_matching_overload", fmt.Sprintf("no overload of method '%s' matches %d arguments", method.Name, len(argTypes)), span)
		return Signature{}, false
	}
	return sig, true
}

func (c *Checker) checkArgTypesWithSignature(args []parser.Expr, sig Signature) []*Type {
	types := make([]*Type, len(args))
	for i, arg := range args {
		if expected, ok := paramTypeForArg(sig, i); ok {
			types[i] = c.checkExprWithExpected(arg, expected)
		} else {
			types[i] = c.checkExpr(arg)
		}
	}
	return types
}

func (c *Checker) inferCallableTypeArgsFromExprs(typeParams []parser.TypeParameter, params []parser.Parameter, args []parser.Expr, baseSubst map[string]*Type) (map[string]*Type, bool) {
	if len(typeParams) == 0 {
		return nil, true
	}
	if len(params) != len(args) {
		return nil, false
	}
	typeParamNames := map[string]bool{}
	for _, param := range typeParams {
		typeParamNames[param.Name] = true
	}
	templateSubst := mergeSubst(c.substForDecl(typeParams, nil), baseSubst)
	inferred := map[string]*Type{}
	for i, param := range params {
		template := c.instantiateTypeRef(param.Type, mergeSubst(inferred, templateSubst))
		contextual := replaceTypeParamsWithUnknown(template, typeParamNames)
		argType := c.checkExprWithExpected(args[i], contextual)
		if !inferTypeArgsFromTypes(argType, template, inferred, typeParamNames) {
			return nil, false
		}
	}
	return inferred, true
}

func (c *Checker) inferCallableTypeArgs(typeParams []parser.TypeParameter, params []parser.Parameter, argTypes []*Type, baseSubst map[string]*Type) (map[string]*Type, bool) {
	if len(typeParams) == 0 {
		return nil, true
	}
	if len(params) != len(argTypes) {
		return nil, false
	}
	typeParamNames := map[string]bool{}
	for _, param := range typeParams {
		typeParamNames[param.Name] = true
	}
	templateSubst := mergeSubst(c.substForDecl(typeParams, nil), baseSubst)
	inferred := map[string]*Type{}
	for i, param := range params {
		template := c.instantiateTypeRef(param.Type, templateSubst)
		if !inferTypeArgsFromTypes(argTypes[i], template, inferred, typeParamNames) {
			return nil, false
		}
	}
	for _, param := range typeParams {
		if _, ok := inferred[param.Name]; !ok {
			return nil, false
		}
	}
	return inferred, true
}

func replaceTypeParamsWithUnknown(t *Type, names map[string]bool) *Type {
	if t == nil {
		return nil
	}
	if t.Kind == TypeParam && names[t.Name] {
		return unknownType
	}
	out := *t
	if len(t.Args) > 0 {
		out.Args = make([]*Type, len(t.Args))
		for i, arg := range t.Args {
			out.Args[i] = replaceTypeParamsWithUnknown(arg, names)
		}
	}
	if t.Signature != nil {
		params := make([]*Type, len(t.Signature.Parameters))
		for i, param := range t.Signature.Parameters {
			params[i] = replaceTypeParamsWithUnknown(param, names)
		}
		sig := *t.Signature
		sig.Parameters = params
		sig.ReturnType = replaceTypeParamsWithUnknown(t.Signature.ReturnType, names)
		out.Signature = &sig
	}
	return &out
}

func inferTypeArgsFromTypes(actual, template *Type, inferred map[string]*Type, typeParams map[string]bool) bool {
	if isUnknown(actual) || isUnknown(template) {
		return true
	}
	if template.Kind == TypeParam && typeParams[template.Name] {
		if existing, ok := inferred[template.Name]; ok {
			return sameType(existing, actual)
		}
		inferred[template.Name] = actual
		return true
	}
	if template.Kind == TypeFunction && actual.Kind == TypeFunction && template.Signature != nil && actual.Signature != nil {
		if len(template.Signature.Parameters) != len(actual.Signature.Parameters) {
			return false
		}
		for i := range template.Signature.Parameters {
			if !inferTypeArgsFromTypes(actual.Signature.Parameters[i], template.Signature.Parameters[i], inferred, typeParams) {
				return false
			}
		}
		return inferTypeArgsFromTypes(actual.Signature.ReturnType, template.Signature.ReturnType, inferred, typeParams)
	}
	if template.Kind == TypeTuple && actual.Kind == TypeTuple && len(template.Args) == len(actual.Args) {
		for i := range template.Args {
			if !inferTypeArgsFromTypes(actual.Args[i], template.Args[i], inferred, typeParams) {
				return false
			}
		}
		return true
	}
	if template.Kind == actual.Kind && template.Name == actual.Name && len(template.Args) == len(actual.Args) {
		for i := range template.Args {
			if !inferTypeArgsFromTypes(actual.Args[i], template.Args[i], inferred, typeParams) {
				return false
			}
		}
	}
	return true
}

func (c *Checker) checkBuiltinConstructorCall(name string, call *parser.CallExpr) *Type {
	if hasNamedCallArgs(call.Args) {
		for _, arg := range call.Args {
			c.checkExpr(arg.Value)
		}
		c.addDiagnostic("invalid_named_argument", "named arguments are not supported for builtin constructors", call.Span)
		return unknownType
	}
	switch name {
	case "List":
		if len(call.Args) == 0 {
			return &Type{Kind: TypeInterface, Name: "List", Args: []*Type{unknownType}}
		}
		elemType := c.checkExpr(call.Args[0].Value)
		for _, arg := range call.Args[1:] {
			argType := c.checkExpr(arg.Value)
			if !sameType(elemType, argType) {
				c.addDiagnostic("type_mismatch", "List constructor arguments must have the same type", exprSpan(arg.Value))
			}
		}
		return &Type{Kind: TypeInterface, Name: "List", Args: []*Type{elemType}}
	case "Set":
		if len(call.Args) == 0 {
			return &Type{Kind: TypeInterface, Name: "Set", Args: []*Type{unknownType}}
		}
		elemType := c.checkExpr(call.Args[0].Value)
		for _, arg := range call.Args[1:] {
			argType := c.checkExpr(arg.Value)
			if !sameType(elemType, argType) {
				c.addDiagnostic("type_mismatch", "Set constructor arguments must have the same type", exprSpan(arg.Value))
			}
		}
		return &Type{Kind: TypeInterface, Name: "Set", Args: []*Type{elemType}}
	case "Map":
		if len(call.Args) == 0 {
			return &Type{Kind: TypeInterface, Name: "Map", Args: []*Type{unknownType, unknownType}}
		}
		keyType := unknownType
		valType := unknownType
		for i, arg := range call.Args {
			pair, ok := arg.Value.(*parser.BinaryExpr)
			if !ok || pair.Operator != ":" {
				c.addDiagnostic("invalid_argument_type", "Map constructor expects key : value pairs", exprSpan(arg.Value))
				c.checkExpr(arg.Value)
				continue
			}
			leftType := c.checkExpr(pair.Left)
			rightType := c.checkExpr(pair.Right)
			if i == 0 {
				keyType, valType = leftType, rightType
				continue
			}
			if !sameType(keyType, leftType) {
				c.addDiagnostic("type_mismatch", "Map constructor keys must have the same type", exprSpan(pair.Left))
			}
			if !sameType(valType, rightType) {
				c.addDiagnostic("type_mismatch", "Map constructor values must have the same type", exprSpan(pair.Right))
			}
		}
		return &Type{Kind: TypeInterface, Name: "Map", Args: []*Type{keyType, valType}}
	case "Array":
		if len(call.Args) == 0 {
			return &Type{Kind: TypeBuiltin, Name: "Array", Args: []*Type{unknownType}}
		}
		elemType := c.checkExpr(call.Args[0].Value)
		for _, arg := range call.Args[1:] {
			argType := c.checkExpr(arg.Value)
			if !sameType(elemType, argType) {
				c.addDiagnostic("type_mismatch", "Array constructor elements must have the same type", exprSpan(arg.Value))
			}
		}
		return &Type{Kind: TypeBuiltin, Name: "Array", Args: []*Type{elemType}}
	case "Range":
		rangeType := &Type{Kind: TypeClass, Name: "IntRange"}
		if _, ok := c.lookupClassInfo("IntRange"); !ok {
			rangeType = &Type{Kind: TypeInterface, Name: "Iterable", Args: []*Type{builtin("Int")}}
		}
		if len(call.Args) != 2 && len(call.Args) != 3 {
			for _, arg := range call.Args {
				c.checkExpr(arg.Value)
			}
			c.addDiagnostic("invalid_argument_count", fmt.Sprintf("Range constructor expects 2 or 3 arguments, got %d", len(call.Args)), call.Span)
			return rangeType
		}
		for _, arg := range call.Args {
			argType := c.checkExpr(arg.Value)
			c.requireAssignable(argType, builtin("Int"), exprSpan(arg.Value), "invalid_argument_type", "Range constructor arguments must be Int")
		}
		return rangeType
	case "Some":
		optionType := &Type{Kind: TypeInterface, Name: "Option", Args: []*Type{unknownType}}
		if _, ok := c.lookupClassInfo("Option"); ok {
			optionType = &Type{Kind: TypeClass, Name: "Option", Args: []*Type{unknownType}}
		}
		if len(call.Args) != 1 {
			for _, arg := range call.Args {
				c.checkExpr(arg.Value)
			}
			c.addDiagnostic("invalid_argument_count", fmt.Sprintf("Some constructor expects 1 argument, got %d", len(call.Args)), call.Span)
			return optionType
		}
		valueType := c.checkExpr(call.Args[0].Value)
		optionType.Args = []*Type{valueType}
		return optionType
	case "None":
		optionType := &Type{Kind: TypeInterface, Name: "Option", Args: []*Type{unknownType}}
		if _, ok := c.lookupClassInfo("Option"); ok {
			optionType = &Type{Kind: TypeClass, Name: "Option", Args: []*Type{unknownType}}
		}
		if len(call.Args) != 0 {
			for _, arg := range call.Args {
				c.checkExpr(arg.Value)
			}
			c.addDiagnostic("invalid_argument_count", fmt.Sprintf("None constructor expects 0 arguments, got %d", len(call.Args)), call.Span)
		}
		return optionType
	case "Ok":
		resultType := &Type{Kind: TypeInterface, Name: "Result", Args: []*Type{unknownType, unknownType}}
		if _, ok := c.classes["Result"]; ok {
			resultType = &Type{Kind: TypeClass, Name: "Result", Args: []*Type{unknownType, unknownType}}
		}
		if len(call.Args) != 1 {
			for _, arg := range call.Args {
				c.checkExpr(arg.Value)
			}
			c.addDiagnostic("invalid_argument_count", fmt.Sprintf("Ok constructor expects 1 argument, got %d", len(call.Args)), call.Span)
			return resultType
		}
		valueType := c.checkExpr(call.Args[0].Value)
		resultType.Args = []*Type{valueType, unknownType}
		return resultType
	case "Err":
		resultType := &Type{Kind: TypeInterface, Name: "Result", Args: []*Type{unknownType, unknownType}}
		if _, ok := c.classes["Result"]; ok {
			resultType = &Type{Kind: TypeClass, Name: "Result", Args: []*Type{unknownType, unknownType}}
		}
		if len(call.Args) != 1 {
			for _, arg := range call.Args {
				c.checkExpr(arg.Value)
			}
			c.addDiagnostic("invalid_argument_count", fmt.Sprintf("Err constructor expects 1 argument, got %d", len(call.Args)), call.Span)
			return resultType
		}
		errorType := c.checkExpr(call.Args[0].Value)
		resultType.Args = []*Type{unknownType, errorType}
		return resultType
	case "Left":
		eitherType := &Type{Kind: TypeInterface, Name: "Either", Args: []*Type{unknownType, unknownType}}
		if _, ok := c.classes["Either"]; ok {
			eitherType = &Type{Kind: TypeClass, Name: "Either", Args: []*Type{unknownType, unknownType}}
		}
		if len(call.Args) != 1 {
			for _, arg := range call.Args {
				c.checkExpr(arg.Value)
			}
			c.addDiagnostic("invalid_argument_count", fmt.Sprintf("Left constructor expects 1 argument, got %d", len(call.Args)), call.Span)
			return eitherType
		}
		leftType := c.checkExpr(call.Args[0].Value)
		eitherType.Args = []*Type{leftType, unknownType}
		return eitherType
	case "Right":
		eitherType := &Type{Kind: TypeInterface, Name: "Either", Args: []*Type{unknownType, unknownType}}
		if _, ok := c.classes["Either"]; ok {
			eitherType = &Type{Kind: TypeClass, Name: "Either", Args: []*Type{unknownType, unknownType}}
		}
		if len(call.Args) != 1 {
			for _, arg := range call.Args {
				c.checkExpr(arg.Value)
			}
			c.addDiagnostic("invalid_argument_count", fmt.Sprintf("Right constructor expects 1 argument, got %d", len(call.Args)), call.Span)
			return eitherType
		}
		rightType := c.checkExpr(call.Args[0].Value)
		eitherType.Args = []*Type{unknownType, rightType}
		return eitherType
	default:
		for _, arg := range call.Args {
			c.checkExpr(arg.Value)
		}
		return unknownType
	}
}

func (c *Checker) checkIndexExpr(expr *parser.IndexExpr) *Type {
	receiverType := c.checkExpr(expr.Receiver)
	indexType := c.checkExpr(expr.Index)
	if isUnknown(receiverType) {
		return unknownType
	}
	if receiverType.Kind == TypeBuiltin && receiverType.Name == "Array" && len(receiverType.Args) == 1 {
		c.requireAssignable(indexType, builtin("Int"), exprSpan(expr.Index), "invalid_index_type", "array index must be Int")
		return receiverType.Args[0]
	}
	if receiverType.Kind == TypeInterface && receiverType.Name == "List" && len(receiverType.Args) == 1 {
		c.requireAssignable(indexType, builtin("Int"), exprSpan(expr.Index), "invalid_index_type", "list index must be Int")
		return receiverType.Args[0]
	}
	if receiverType.Kind == TypeInterface && receiverType.Name == "Map" && len(receiverType.Args) == 2 {
		expectedKey := receiverType.Args[0]
		if !sameType(indexType, expectedKey) {
			c.addDiagnostic("type_mismatch", "map index must have key type "+expectedKey.String(), expr.Span)
		}
		return c.optionType(receiverType.Args[1])
	}
	if result, ok := c.resolveOperatorExprType(receiverType, "[]", []*Type{indexType}, expr.Span); ok {
		return result
	}
	c.addDiagnostic("invalid_index_target", "indexing requires Array[T], List[T], Map[K, V], or operator []", expr.Span)
	return unknownType
}

func (c *Checker) checkRecordUpdateExpr(expr *parser.RecordUpdateExpr) *Type {
	receiverType := c.checkExpr(expr.Receiver)
	if isUnknown(receiverType) {
		for _, update := range expr.Updates {
			c.checkExpr(update.Value)
		}
		return unknownType
	}
	if receiverType.Kind != TypeClass {
		for _, update := range expr.Updates {
			c.checkExpr(update.Value)
		}
		c.addDiagnostic("invalid_record_update", "update requires a record or class value", expr.Span)
		return unknownType
	}
	info, ok := c.classes[receiverType.Name]
	if !ok || info.decl.Object || info.decl.Enum {
		for _, update := range expr.Updates {
			c.checkExpr(update.Value)
		}
		c.addDiagnostic("invalid_record_update", "update requires a record or class value", expr.Span)
		return unknownType
	}
	if !info.decl.Record {
		for _, field := range info.decl.Fields {
			if field.Private {
				for _, update := range expr.Updates {
					c.checkExpr(update.Value)
				}
				c.addDiagnostic("invalid_record_update", "class update requires a class without private fields", expr.Span)
				return unknownType
			}
		}
	}
	subst := c.substForDecl(info.decl.TypeParameters, receiverType.Args)
	seen := map[string]bool{}
	for _, update := range expr.Updates {
		if seen[update.Name] {
			c.addDiagnostic("invalid_record_update", "duplicate updated field '"+update.Name+"'", expr.Span)
			c.checkExpr(update.Value)
			continue
		}
		seen[update.Name] = true
		field, ok := info.fields[update.Name]
		if !ok {
			c.addDiagnostic("unknown_member", "unknown field '"+update.Name+"'", expr.Span)
			c.checkExpr(update.Value)
			continue
		}
		if field.decl.Private && !c.canAccessPrivate(info.decl) {
			c.addDiagnostic("private_access", "cannot access private field '"+update.Name+"' outside class '"+info.decl.Name+"'", expr.Span)
			c.checkExpr(update.Value)
			continue
		}
		expected := c.instantiateTypeRef(field.decl.Type, subst)
		valueType := c.checkExprWithExpected(update.Value, expected)
		c.requireAssignable(valueType, expected, exprSpan(update.Value), "type_mismatch", "cannot assign "+valueType.String()+" to "+expected.String())
	}
	return receiverType
}

func (c *Checker) checkAnonymousRecordExpr(expr *parser.AnonymousRecordExpr, expected *Type) *Type {
	if len(expr.Values) > 0 {
		if expected == nil || expected.Kind != TypeRecord {
			for _, value := range expr.Values {
				c.checkExpr(value)
			}
			c.addDiagnostic("invalid_record_literal", "positional record(...) requires an expected anonymous record shape", expr.Span)
			return unknownType
		}
		if len(expr.Values) != len(expected.Fields) {
			for _, value := range expr.Values {
				c.checkExpr(value)
			}
			c.addDiagnostic("invalid_record_literal", fmt.Sprintf("record(...) expects %d values for shape %s, got %d", len(expected.Fields), expected.String(), len(expr.Values)), expr.Span)
			return expected
		}
		fields := make([]RecordField, len(expected.Fields))
		expr.Fields = make([]parser.CallArg, len(expected.Fields))
		for i, field := range expected.Fields {
			valueType := c.checkExprWithExpected(expr.Values[i], field.Type)
			c.requireAssignable(valueType, field.Type, exprSpan(expr.Values[i]), "type_mismatch", "cannot assign "+valueType.String()+" to "+field.Type.String())
			fields[i] = RecordField{Name: field.Name, Type: field.Type}
			expr.Fields[i] = parser.CallArg{
				Name:  field.Name,
				Value: expr.Values[i],
				Span: parser.Span{
					Start: expr.Span.Start,
					End:   exprSpan(expr.Values[i]).End,
				},
			}
		}
		expr.Values = nil
		return &Type{Kind: TypeRecord, Name: "Record", Fields: fields}
	}
	fields := make([]RecordField, len(expr.Fields))
	seen := map[string]bool{}
	expectedByName := map[string]*Type{}
	if expected != nil && expected.Kind == TypeRecord {
		for _, field := range expected.Fields {
			expectedByName[field.Name] = field.Type
		}
	}
	matchesExpected := expected != nil && expected.Kind == TypeRecord && len(expected.Fields) == len(expr.Fields)
	for i, field := range expr.Fields {
		if seen[field.Name] {
			c.addDiagnostic("duplicate_record_field", "duplicate record field '"+field.Name+"'", field.Span)
		}
		seen[field.Name] = true
		expectedFieldType, hasExpectedField := expectedByName[field.Name]
		valueType := c.checkExprWithExpected(field.Value, expectedFieldType)
		if hasExpectedField {
			c.requireAssignable(valueType, expectedFieldType, exprSpan(field.Value), "type_mismatch", "cannot assign "+valueType.String()+" to "+expectedFieldType.String())
			fields[i] = RecordField{Name: field.Name, Type: expectedFieldType}
			continue
		}
		matchesExpected = false
		fields[i] = RecordField{Name: field.Name, Type: valueType}
	}
	if matchesExpected {
		for _, field := range expected.Fields {
			if !seen[field.Name] {
				matchesExpected = false
				break
			}
		}
		if matchesExpected {
			return expected
		}
	}
	return &Type{Kind: TypeRecord, Name: "Record", Fields: fields}
}

func (c *Checker) checkAnonymousInterfaceExpr(expr *parser.AnonymousInterfaceExpr) *Type {
	c.anonClassID++
	name := fmt.Sprintf("__anon_iface_%d", c.anonClassID)
	decl := &parser.ClassDecl{
		Name:       name,
		Implements: expr.Interfaces,
		Methods:    expr.Methods,
		Span:       expr.Span,
	}
	info := classInfo{
		name:      name,
		decl:      decl,
		fields:    map[string]fieldInfo{},
		methods:   map[string][]methodInfo{},
		enumCases: map[string]parser.EnumCaseDecl{},
	}
	for _, method := range expr.Methods {
		info.methods[method.Name] = append(info.methods[method.Name], methodInfo{decl: method})
		if method.Constructor {
			info.constructors = append(info.constructors, method)
		}
	}
	c.classes[name] = info

	for _, impl := range expr.Interfaces {
		implType := c.resolveDeclaredType(impl)
		if implType.Kind != TypeInterface {
			c.addDiagnostic("invalid_anonymous_interface", "anonymous object can only implement interfaces", impl.Span)
		}
	}
	for _, method := range expr.Methods {
		c.checkMethod(method, decl)
	}
	c.checkOperatorMethods(info)
	for _, impl := range expr.Interfaces {
		c.checkInterfaceImplementation(info, impl)
	}
	return &Type{Kind: TypeClass, Name: name}
}

func (c *Checker) checkConstructorCall(class classInfo, call *parser.CallExpr) *Type {
	classType := &Type{Kind: TypeClass, Name: class.name}
	if class.decl.Object {
		c.addDiagnostic("invalid_call_target", "object '"+class.decl.Name+"' is a singleton and cannot be called as a constructor", call.Span)
		for _, arg := range call.Args {
			c.checkExpr(arg.Value)
		}
		return classType
	}
	orderedArgs := callArgValues(call.Args)
	if hasNamedCallArgs(call.Args) {
		if ctor, reordered, ok := c.resolveNamedConstructorOverload(class, call.Args, call.Span); ok {
			orderedArgs = reordered
			sig := c.primaryConstructorSignature(class.decl)
			if ctor != nil {
				sig = c.instantiateMethodSignature(ctor, class.decl, constructorTypeArgs(class.decl, call.Callee))
			}
			for i := range orderedArgs {
				if expected, ok := paramTypeForArg(sig, i); ok {
					argType := c.checkExprWithExpected(orderedArgs[i], expected)
					c.requireAssignable(argType, expected, exprSpan(orderedArgs[i]), "invalid_argument_type", "cannot pass "+argType.String()+" to parameter of type "+expected.String())
				} else {
					c.checkExpr(orderedArgs[i])
				}
			}
		}
	} else {
		if len(orderedArgs) == 1 {
			if recordExpr, ok := orderedArgs[0].(*parser.AnonymousRecordExpr); ok {
				if len(recordExpr.Values) > 0 {
					c.resolvePositionalRecordShapeConstruction(class, recordExpr, call.Span)
					return classType
				}
			}
		}
		argTypes := c.checkArgTypes(orderedArgs)
		if len(orderedArgs) == 1 && len(argTypes) == 1 && argTypes[0].Kind == TypeRecord {
			c.resolveRecordShapeConstruction(class, argTypes[0], call.Span)
			c.checkExpr(orderedArgs[0])
			if ident, ok := call.Callee.(*parser.Identifier); ok {
				if refType, ok := c.lookupTypeInstance(ident.Name); ok {
					classType = refType
				}
			}
			return classType
		}
		if ctor, ok := c.resolveConstructorOverload(class, argTypes, call.Span); ok {
			sig := c.primaryConstructorSignature(class.decl)
			if ctor != nil {
				sig = c.instantiateMethodSignature(ctor, class.decl, constructorTypeArgs(class.decl, call.Callee))
			}
			for i := range orderedArgs {
				if expected, ok := paramTypeForArg(sig, i); ok {
					argType := c.checkExprWithExpected(orderedArgs[i], expected)
					c.requireAssignable(argType, expected, exprSpan(orderedArgs[i]), "invalid_argument_type", "cannot pass "+argType.String()+" to parameter of type "+expected.String())
				} else {
					c.checkExpr(orderedArgs[i])
				}
			}
		}
	}
	if ident, ok := call.Callee.(*parser.Identifier); ok {
		if refType, ok := c.lookupTypeInstance(ident.Name); ok {
			classType = refType
		}
	}
	return classType
}

func (c *Checker) resolveRecordShapeConstruction(class classInfo, actual *Type, span parser.Span) bool {
	required, optional, ok := classAnonymousRecordShape(class.decl, nil, c)
	if !ok {
		c.addDiagnostic("no_matching_overload", "class/record '"+class.decl.Name+"' cannot be built from an anonymous record because it has private fields without initializers", span)
		return false
	}
	if recordMatchesVisibleShape(actual.Fields, required, optional) {
		return true
	}
	c.addDiagnostic("no_matching_overload", "class/record '"+class.decl.Name+"' requires an anonymous record with exactly matching field names and types", span)
	return false
}

func (c *Checker) resolvePositionalRecordShapeConstruction(class classInfo, actual *parser.AnonymousRecordExpr, span parser.Span) bool {
	required, optional, ok := classAnonymousRecordShape(class.decl, nil, c)
	if !ok {
		c.addDiagnostic("no_matching_overload", "class/record '"+class.decl.Name+"' cannot be built from an anonymous record because it has private fields without initializers", span)
		return false
	}
	expected := append(append([]RecordField{}, required...), optional...)
	if len(actual.Values) < len(required) || len(actual.Values) > len(expected) {
		c.addDiagnostic("no_matching_overload", "class/record '"+class.decl.Name+"' requires an anonymous record with exactly matching field names and types", span)
		return false
	}
	for i, field := range expected {
		if i >= len(actual.Values) {
			break
		}
		valueType := c.checkExprWithExpected(actual.Values[i], field.Type)
		if !sameType(valueType, field.Type) {
			c.addDiagnostic("no_matching_overload", "class/record '"+class.decl.Name+"' requires an anonymous record with exactly matching field names and types", span)
			return false
		}
	}
	for i := len(actual.Values); i < len(expected); i++ {
		if i < len(required) {
			c.addDiagnostic("no_matching_overload", "class/record '"+class.decl.Name+"' requires an anonymous record with exactly matching field names and types", span)
			return false
		}
	}
	return true
}

func (c *Checker) checkMethodCall(member *parser.MemberExpr, args []parser.CallArg) *Type {
	if ident, ok := member.Receiver.(*parser.Identifier); ok {
		if enumSig, ok := c.tryEnumCaseCallFromIdentifier(ident.Name, member.Name, args, member.Span); ok {
			return enumSig
		}
		if ident.Name == "Array" && member.Name == "ofLength" {
			if hasNamedCallArgs(args) {
				c.checkArgTypes(callArgValues(args))
				c.addDiagnostic("invalid_named_argument", "named arguments are not supported for Array.ofLength", member.Span)
				return &Type{Kind: TypeBuiltin, Name: "Array", Args: []*Type{unknownType}}
			}
			orderedArgs := callArgValues(args)
			if len(orderedArgs) != 1 {
				c.checkArgTypes(orderedArgs)
				c.addDiagnostic("invalid_argument_count", fmt.Sprintf("method '%s' expects %d arguments, got %d", member.Name, 1, len(orderedArgs)), member.Span)
				return &Type{Kind: TypeBuiltin, Name: "Array", Args: []*Type{unknownType}}
			}
			argType := c.checkExpr(orderedArgs[0])
			c.requireAssignable(argType, builtin("Int"), exprSpan(orderedArgs[0]), "invalid_argument_type", "Array.ofLength length must be Int")
			return &Type{Kind: TypeBuiltin, Name: "Array", Args: []*Type{unknownType}}
		}
	}
	receiverType := c.checkExpr(member.Receiver)
	if isUnknown(receiverType) {
		c.checkArgTypes(callArgValues(args))
		return unknownType
	}
	if receiverType.Kind == TypeModule {
		info, ok := c.imports[receiverType.Name]
		if !ok {
			c.checkArgTypes(callArgValues(args))
			return unknownType
		}
		if fnDecl, ok := info.functionDecls[member.Name]; ok {
			orderedArgs := callArgValues(args)
			if hasNamedCallArgs(args) {
				reordered, ok := c.reorderCallArgs(fnDecl.Parameters, args, member.Span, "function '"+member.Name+"'")
				if !ok {
					c.checkArgTypes(callArgValues(args))
					return c.instantiateFunctionSignature(fnDecl, nil).ReturnType
				}
				orderedArgs = reordered
			}
			if len(fnDecl.TypeParameters) == 0 {
				fn := info.functions[member.Name]
				if !validArgCount(fn, len(orderedArgs)) {
					c.addDiagnostic("invalid_argument_count", fmt.Sprintf("call expects %s arguments, got %d", expectedArgCount(fn), len(orderedArgs)), member.Span)
				}
				for i, arg := range orderedArgs {
					if expected, ok := paramTypeForArg(fn, i); ok {
						argType := c.checkExprWithExpected(arg, expected)
						c.requireAssignable(argType, expected, exprSpan(arg), "invalid_argument_type", "cannot pass "+argType.String()+" to parameter of type "+expected.String())
					} else {
						c.checkExpr(arg)
					}
				}
				return fn.ReturnType
			}
			sig, ok := c.resolveFunctionCallSignature(fnDecl, orderedArgs, member.Span)
			if !ok {
				return unknownType
			}
			if !validArgCount(sig, len(orderedArgs)) {
				c.addDiagnostic("invalid_argument_count", fmt.Sprintf("call expects %s arguments, got %d", expectedArgCount(sig), len(orderedArgs)), member.Span)
			}
			for i, arg := range orderedArgs {
				if expected, ok := paramTypeForArg(sig, i); ok {
					argType := c.checkExprWithExpected(arg, expected)
					c.requireAssignable(argType, expected, exprSpan(arg), "invalid_argument_type", "cannot pass "+argType.String()+" to parameter of type "+expected.String())
				} else {
					c.checkExpr(arg)
				}
			}
			return sig.ReturnType
		}
		if class, ok := info.classes[member.Name]; ok {
			call := &parser.CallExpr{Callee: member, Args: args, Span: member.Span}
			return c.checkConstructorCall(class, call)
		}
		if object, ok := info.objects[member.Name]; ok {
			c.checkArgTypes(callArgValues(args))
			c.addDiagnostic("invalid_call_target", "object '"+object.decl.Name+"' is not callable", member.Span)
			return unknownType
		}
		c.checkArgTypes(callArgValues(args))
		c.addDiagnostic("unknown_member", "unknown imported member '"+member.Name+"' on module '"+receiverType.Name+"'", member.Span)
		return unknownType
	}
	if receiverType.Kind == TypeBuiltin && receiverType.Name == "Array" {
		if hasNamedCallArgs(args) {
			c.checkArgTypes(callArgValues(args))
			c.addDiagnostic("invalid_named_argument", "named arguments are not supported for Array methods", member.Span)
			return unknownType
		}
		orderedArgs := callArgValues(args)
		switch member.Name {
		case "get":
			argTypes := c.checkArgTypes(orderedArgs)
			if len(argTypes) != 1 {
				c.addDiagnostic("invalid_argument_count", fmt.Sprintf("method '%s' expects %d arguments, got %d", member.Name, 1, len(argTypes)), member.Span)
				return c.optionType(unknownType)
			}
			c.requireAssignable(argTypes[0], builtin("Int"), exprSpan(orderedArgs[0]), "invalid_argument_type", "get index must be Int")
			elemType := unknownType
			if len(receiverType.Args) == 1 {
				elemType = receiverType.Args[0]
			}
			return c.optionType(elemType)
		case "first":
			argTypes := c.checkArgTypes(orderedArgs)
			if len(argTypes) != 0 {
				c.addDiagnostic("invalid_argument_count", fmt.Sprintf("method '%s' expects %d arguments, got %d", member.Name, 0, len(argTypes)), member.Span)
			}
			elemType := unknownType
			if len(receiverType.Args) == 1 {
				elemType = receiverType.Args[0]
			}
			return c.optionType(elemType)
		case "last":
			argTypes := c.checkArgTypes(orderedArgs)
			if len(argTypes) != 0 {
				c.addDiagnostic("invalid_argument_count", fmt.Sprintf("method '%s' expects %d arguments, got %d", member.Name, 0, len(argTypes)), member.Span)
			}
			elemType := unknownType
			if len(receiverType.Args) == 1 {
				elemType = receiverType.Args[0]
			}
			return c.optionType(elemType)
		case "map":
			if len(orderedArgs) != 1 {
				c.addDiagnostic("invalid_argument_count", fmt.Sprintf("method '%s' expects %d arguments, got %d", member.Name, 1, len(orderedArgs)), member.Span)
				return &Type{Kind: TypeBuiltin, Name: "Array", Args: []*Type{unknownType}}
			}
			elemType := unknownType
			if len(receiverType.Args) == 1 {
				elemType = receiverType.Args[0]
			}
			expected := functionType("map", Signature{Parameters: []*Type{elemType}, ReturnType: unknownType})
			mappedType := c.checkExprWithExpected(orderedArgs[0], expected)
			if mappedType.Kind != TypeFunction || mappedType.Signature == nil || len(mappedType.Signature.Parameters) != 1 {
				c.addDiagnostic("invalid_argument_type", "map expects parameter of type T -> X", exprSpan(orderedArgs[0]))
				return &Type{Kind: TypeBuiltin, Name: "Array", Args: []*Type{unknownType}}
			}
			c.requireAssignable(mappedType.Signature.Parameters[0], elemType, exprSpan(orderedArgs[0]), "invalid_argument_type", "map lambda must accept "+elemType.String())
			return &Type{Kind: TypeBuiltin, Name: "Array", Args: []*Type{mappedType.Signature.ReturnType}}
		case "exists":
			if len(orderedArgs) != 1 {
				c.addDiagnostic("invalid_argument_count", fmt.Sprintf("method '%s' expects %d arguments, got %d", member.Name, 1, len(orderedArgs)), member.Span)
				return builtin("Bool")
			}
			elemType := unknownType
			if len(receiverType.Args) == 1 {
				elemType = receiverType.Args[0]
			}
			expected := functionType("exists", Signature{Parameters: []*Type{elemType}, ReturnType: builtin("Bool")})
			predicateType := c.checkExprWithExpected(orderedArgs[0], expected)
			if predicateType.Kind != TypeFunction || predicateType.Signature == nil || len(predicateType.Signature.Parameters) != 1 {
				c.addDiagnostic("invalid_argument_type", "exists expects parameter of type T -> Bool", exprSpan(orderedArgs[0]))
				return builtin("Bool")
			}
			c.requireAssignable(predicateType.Signature.Parameters[0], elemType, exprSpan(orderedArgs[0]), "invalid_argument_type", "exists lambda must accept "+elemType.String())
			c.requireAssignable(predicateType.Signature.ReturnType, builtin("Bool"), exprSpan(orderedArgs[0]), "invalid_argument_type", "exists lambda must return Bool")
			return builtin("Bool")
		case "forAll":
			if len(orderedArgs) != 1 {
				c.addDiagnostic("invalid_argument_count", fmt.Sprintf("method '%s' expects %d arguments, got %d", member.Name, 1, len(orderedArgs)), member.Span)
				return builtin("Bool")
			}
			elemType := unknownType
			if len(receiverType.Args) == 1 {
				elemType = receiverType.Args[0]
			}
			expected := functionType("forAll", Signature{Parameters: []*Type{elemType}, ReturnType: builtin("Bool")})
			predicateType := c.checkExprWithExpected(orderedArgs[0], expected)
			if predicateType.Kind != TypeFunction || predicateType.Signature == nil || len(predicateType.Signature.Parameters) != 1 {
				c.addDiagnostic("invalid_argument_type", "forAll expects parameter of type T -> Bool", exprSpan(orderedArgs[0]))
				return builtin("Bool")
			}
			c.requireAssignable(predicateType.Signature.Parameters[0], elemType, exprSpan(orderedArgs[0]), "invalid_argument_type", "forAll lambda must accept "+elemType.String())
			c.requireAssignable(predicateType.Signature.ReturnType, builtin("Bool"), exprSpan(orderedArgs[0]), "invalid_argument_type", "forAll lambda must return Bool")
			return builtin("Bool")
		case "forEach":
			if len(orderedArgs) != 1 {
				c.addDiagnostic("invalid_argument_count", fmt.Sprintf("method '%s' expects %d arguments, got %d", member.Name, 1, len(orderedArgs)), member.Span)
				return builtin("Unit")
			}
			elemType := unknownType
			if len(receiverType.Args) == 1 {
				elemType = receiverType.Args[0]
			}
			expected := functionType("forEach", Signature{Parameters: []*Type{elemType}, ReturnType: builtin("Unit")})
			callbackType := c.checkExprWithExpected(orderedArgs[0], expected)
			if callbackType.Kind != TypeFunction || callbackType.Signature == nil || len(callbackType.Signature.Parameters) != 1 {
				c.addDiagnostic("invalid_argument_type", "forEach expects parameter of type T -> Unit", exprSpan(orderedArgs[0]))
				return builtin("Unit")
			}
			c.requireAssignable(callbackType.Signature.Parameters[0], elemType, exprSpan(orderedArgs[0]), "invalid_argument_type", "forEach lambda must accept "+elemType.String())
			return builtin("Unit")
		case "size":
			argTypes := c.checkArgTypes(orderedArgs)
			if len(argTypes) != 0 {
				c.addDiagnostic("invalid_argument_count", fmt.Sprintf("method '%s' expects %d arguments, got %d", member.Name, 0, len(argTypes)), member.Span)
			}
			return builtin("Int")
		case "zip":
			argTypes := c.checkArgTypes(orderedArgs)
			if len(argTypes) != 1 {
				c.addDiagnostic("invalid_argument_count", fmt.Sprintf("method '%s' expects %d arguments, got %d", member.Name, 1, len(argTypes)), member.Span)
				return &Type{Kind: TypeBuiltin, Name: "Array", Args: []*Type{unknownType}}
			}
			if argTypes[0].Kind != TypeBuiltin || argTypes[0].Name != "Array" || len(argTypes[0].Args) != 1 {
				c.addDiagnostic("invalid_argument_type", "zip expects parameter of type Array[T]", member.Span)
				return &Type{Kind: TypeBuiltin, Name: "Array", Args: []*Type{unknownType}}
			}
			elemType := unknownType
			if len(receiverType.Args) == 1 {
				elemType = receiverType.Args[0]
			}
			return &Type{Kind: TypeBuiltin, Name: "Array", Args: []*Type{{Kind: TypeTuple, Name: "Tuple", Args: []*Type{elemType, argTypes[0].Args[0]}}}}
		case "zipWithIndex":
			argTypes := c.checkArgTypes(orderedArgs)
			if len(argTypes) != 0 {
				c.addDiagnostic("invalid_argument_count", fmt.Sprintf("method '%s' expects %d arguments, got %d", member.Name, 0, len(argTypes)), member.Span)
			}
			elemType := unknownType
			if len(receiverType.Args) == 1 {
				elemType = receiverType.Args[0]
			}
			return &Type{Kind: TypeBuiltin, Name: "Array", Args: []*Type{{Kind: TypeTuple, Name: "Tuple", Args: []*Type{elemType, builtin("Int")}}}}
		default:
			c.addDiagnostic("unknown_member", "unknown member '"+member.Name+"'", member.Span)
			return unknownType
		}
	}
	if receiverType.Kind == TypeBuiltin && receiverType.Name == "Str" {
		if hasNamedCallArgs(args) {
			c.checkArgTypes(callArgValues(args))
			c.addDiagnostic("invalid_named_argument", "named arguments are not supported for Str methods", member.Span)
			return unknownType
		}
		argTypes := c.checkArgTypes(callArgValues(args))
		switch member.Name {
		case "size":
			if len(argTypes) != 0 {
				c.addDiagnostic("invalid_argument_count", fmt.Sprintf("method '%s' expects %d arguments, got %d", member.Name, 0, len(argTypes)), member.Span)
			}
			return builtin("Int")
		case "split":
			if len(argTypes) != 1 {
				c.addDiagnostic("invalid_argument_count", fmt.Sprintf("method '%s' expects %d arguments, got %d", member.Name, 1, len(argTypes)), member.Span)
				return &Type{Kind: TypeBuiltin, Name: "Array", Args: []*Type{builtin("Str")}}
			}
			c.requireAssignable(argTypes[0], builtin("Str"), exprSpan(callArgValues(args)[0]), "invalid_argument_type", "split expects separator of type Str")
			return &Type{Kind: TypeBuiltin, Name: "Array", Args: []*Type{builtin("Str")}}
		case "indexOf":
			if len(argTypes) != 1 {
				c.addDiagnostic("invalid_argument_count", fmt.Sprintf("method '%s' expects %d arguments, got %d", member.Name, 1, len(argTypes)), member.Span)
				return builtin("Int")
			}
			c.requireAssignable(argTypes[0], builtin("Str"), exprSpan(callArgValues(args)[0]), "invalid_argument_type", "indexOf expects substring of type Str")
			return builtin("Int")
		default:
			c.addDiagnostic("unknown_member", "unknown member '"+member.Name+"'", member.Span)
			return unknownType
		}
	}
	switch receiverType.Kind {
	case TypeClass:
		info, ok := c.lookupClassInfo(receiverType.Name)
		if !ok {
			c.checkArgTypes(callArgValues(args))
			return unknownType
		}
		if info.decl.Enum {
			if enumCase, ok := info.enumCases[member.Name]; ok {
				params := make([]parser.Parameter, len(enumCase.Fields))
				sig := Signature{Parameters: make([]*Type, len(enumCase.Fields)), ReturnType: receiverType}
				for i, field := range enumCase.Fields {
					params[i] = parser.Parameter{Name: field.Name, Type: field.Type, Span: field.Span}
					sig.Parameters[i] = c.resolveDeclaredType(field.Type)
				}
				orderedArgs := callArgValues(args)
				if hasNamedCallArgs(args) {
					reordered, ok := c.reorderCallArgs(params, args, member.Span, "enum case '"+member.Name+"'")
					if !ok {
						c.checkArgTypes(callArgValues(args))
						return receiverType
					}
					orderedArgs = reordered
				}
				if !validArgCount(sig, len(orderedArgs)) {
					c.addDiagnostic("invalid_argument_count", fmt.Sprintf("enum case '%s' expects %s arguments, got %d", member.Name, expectedArgCount(sig), len(orderedArgs)), member.Span)
				}
				for i := range orderedArgs {
					if expected, ok := paramTypeForArg(sig, i); ok {
						argType := c.checkExprWithExpected(orderedArgs[i], expected)
						c.requireAssignable(argType, expected, exprSpan(orderedArgs[i]), "invalid_argument_type", "cannot pass "+argType.String()+" to parameter of type "+expected.String())
					} else {
						c.checkExpr(orderedArgs[i])
					}
				}
				return receiverType
			}
		}
		var (
			method      methodInfo
			okMethod    bool
			orderedArgs []parser.Expr
		)
		if receiverType.Name == "OS" && (member.Name == "println" || member.Name == "print" || member.Name == "printf" || member.Name == "panic") {
			if hasNamedCallArgs(args) {
				c.addDiagnostic("invalid_named_argument", "named arguments are not supported for variadic methods", member.Span)
				c.checkArgTypes(callArgValues(args))
				return unknownType
			}
			orderedArgs = callArgValues(args)
			for _, arg := range orderedArgs {
				c.checkExpr(arg)
			}
			if member.Name == "panic" {
				return unknownType
			}
			return builtin("Unit")
		}
		if !hasNamedCallArgs(args) && !c.classHasDefaultInterfaceMethod(info, member.Name) {
			if methods := info.methods[member.Name]; len(methods) == 1 {
				if methods[0].decl.Private && !c.canAccessPrivate(info.decl) {
					c.addDiagnostic("private_access", "cannot access private method '"+member.Name+"' outside class '"+info.decl.Name+"'", member.Span)
					return unknownType
				}
				orderedArgs = callArgValues(args)
				sig, ok := c.resolveMethodCallSignature(info, receiverType, methods[0].decl, orderedArgs, member.Span)
				if ok {
					c.checkCallArgsAgainstSignature(orderedArgs, sig)
					return sig.ReturnType
				}
				return unknownType
			}
		}
		if hasNamedCallArgs(args) {
			method, orderedArgs, okMethod = c.tryResolveNamedMethodOverload(info, receiverType, member.Name, args)
			if !okMethod {
				sig, ifaceArgs, ok := c.tryResolveNamedDefaultInterfaceMethod(receiverType, info, member.Name, args, member.Span)
				if ok {
					c.checkCallArgsAgainstSignature(ifaceArgs, sig)
					return sig.ReturnType
				}
				method, orderedArgs, okMethod = c.resolveNamedMethodOverload(info, receiverType, member.Name, args, member.Span)
			}
		} else {
			orderedArgs = callArgValues(args)
			argTypes := c.checkArgTypes(orderedArgs)
			method, okMethod = c.tryResolveMethodOverload(info, receiverType, member.Name, argTypes)
			if !okMethod {
				sig, ok := c.tryResolveDefaultInterfaceMethod(receiverType, info, member.Name, orderedArgs, member.Span)
				if ok {
					c.checkCallArgsAgainstSignature(orderedArgs, sig)
					return sig.ReturnType
				}
				method, okMethod = c.resolveMethodOverload(info, receiverType, member.Name, argTypes, member.Span)
			}
		}
		if !okMethod {
			return unknownType
		}
		sig, ok := c.resolveMethodCallSignature(info, receiverType, method.decl, orderedArgs, member.Span)
		if !ok {
			return unknownType
		}
		for i := range orderedArgs {
			if expected, ok := paramTypeForArg(sig, i); ok {
				argType := c.checkExprWithExpected(orderedArgs[i], expected)
				c.requireAssignable(argType, expected, exprSpan(orderedArgs[i]), "invalid_argument_type", "cannot pass "+argType.String()+" to parameter of type "+expected.String())
			} else {
				c.checkExpr(orderedArgs[i])
			}
		}
		return sig.ReturnType
	case TypeObject:
		info, ok := c.lookupObjectInfo(receiverType.Name)
		if !ok {
			c.checkArgTypes(callArgValues(args))
			return unknownType
		}
		var (
			method      methodInfo
			okMethod    bool
			orderedArgs []parser.Expr
		)
		if receiverType.Name == "OS" && (member.Name == "println" || member.Name == "print" || member.Name == "printf" || member.Name == "panic") {
			if hasNamedCallArgs(args) {
				c.addDiagnostic("invalid_named_argument", "named arguments are not supported for variadic methods", member.Span)
				c.checkArgTypes(callArgValues(args))
				return unknownType
			}
			orderedArgs = callArgValues(args)
			for _, arg := range orderedArgs {
				c.checkExpr(arg)
			}
			if member.Name == "panic" {
				return unknownType
			}
			return builtin("Unit")
		}
		if hasNamedCallArgs(args) {
			method, orderedArgs, okMethod = c.resolveNamedMethodOverload(info, &Type{Kind: TypeObject, Name: info.name}, member.Name, args, member.Span)
		} else {
			orderedArgs = callArgValues(args)
			argTypes := c.checkArgTypes(orderedArgs)
			method, okMethod = c.resolveMethodOverload(info, &Type{Kind: TypeObject, Name: info.name}, member.Name, argTypes, member.Span)
		}
		if !okMethod {
			return unknownType
		}
		sig, ok := c.resolveMethodCallSignature(info, &Type{Kind: TypeObject, Name: info.name}, method.decl, orderedArgs, member.Span)
		if !ok {
			return unknownType
		}
		c.checkCallArgsAgainstSignature(orderedArgs, sig)
		return sig.ReturnType
	case TypeInterface:
		info, ok := c.interfaces[receiverType.Name]
		if !ok {
			c.checkArgTypes(callArgValues(args))
			return unknownType
		}
		method, ok := c.lookupInterfaceMethodInfo(info.decl, member.Name, map[string]bool{})
		if !ok {
			c.addDiagnostic("unknown_member", "unknown member '"+member.Name+"'", member.Span)
			return unknownType
		}
		orderedArgs := callArgValues(args)
		if hasNamedCallArgs(args) {
			if method.decl.Parameters[len(method.decl.Parameters)-1].Variadic {
				c.checkArgTypes(callArgValues(args))
				c.addDiagnostic("invalid_named_argument", "named arguments are not supported for variadic methods", member.Span)
				return unknownType
			}
			reordered, ok := c.reorderCallArgs(method.decl.Parameters, args, member.Span, "method '"+member.Name+"'")
			if !ok {
				c.checkArgTypes(callArgValues(args))
				return unknownType
			}
			orderedArgs = reordered
		}
		if receiverType.Name == "Printer" && (member.Name == "println" || member.Name == "print" || member.Name == "printf") {
			for _, arg := range orderedArgs {
				c.checkExpr(arg)
			}
			return builtin("Unit")
		}
		sig, ok := c.resolveInterfaceMethodCallSignature(info, receiverType, method.decl, orderedArgs, member.Span)
		if !ok {
			return unknownType
		}
		if !validArgCount(sig, len(orderedArgs)) {
			c.addDiagnostic("invalid_argument_count", fmt.Sprintf("method '%s' expects %s arguments, got %d", member.Name, expectedArgCount(sig), len(orderedArgs)), member.Span)
		}
		for i := range orderedArgs {
			if expected, ok := paramTypeForArg(sig, i); ok {
				argType := c.checkExprWithExpected(orderedArgs[i], expected)
				c.requireAssignable(argType, expected, exprSpan(orderedArgs[i]), "invalid_argument_type", "cannot pass "+argType.String()+" to parameter of type "+expected.String())
			} else {
				c.checkExpr(orderedArgs[i])
			}
		}
		return sig.ReturnType
	default:
		c.addDiagnostic("invalid_call_target", "member call requires class or interface receiver", member.Span)
		return unknownType
	}
}

func (c *Checker) checkCallArgsAgainstSignature(args []parser.Expr, sig Signature) {
	for i := range args {
		if expected, ok := paramTypeForArg(sig, i); ok {
			argType := c.checkExprWithExpected(args[i], expected)
			c.requireAssignable(argType, expected, exprSpan(args[i]), "invalid_argument_type", "cannot pass "+argType.String()+" to parameter of type "+expected.String())
		} else {
			c.checkExpr(args[i])
		}
	}
}

func (c *Checker) checkLambdaExpr(expr *parser.LambdaExpr, expected *Type) *Type {
	c.pushScope()
	defer c.popScope()

	boundary := len(c.scopes) - 1
	c.lambdaScopes = append(c.lambdaScopes, boundary)
	defer func() { c.lambdaScopes = c.lambdaScopes[:len(c.lambdaScopes)-1] }()

	params := make([]*Type, len(expr.Parameters))
	expectedSig := (*Signature)(nil)
	destructuredTupleArg := false
	if expected != nil && expected.Kind == TypeFunction && expected.Signature != nil {
		expectedSig = expected.Signature
		if len(expectedSig.Parameters) != len(expr.Parameters) {
			if len(expectedSig.Parameters) == 1 && len(expr.Parameters) > 1 {
				parts, _, ok := c.destructurableValueTypes(expectedSig.Parameters[0])
				if ok && len(parts) == len(expr.Parameters) {
					destructuredTupleArg = true
					params = append([]*Type(nil), parts...)
				} else {
					c.addDiagnostic("invalid_lambda_type", "lambda parameter count does not match expected function type", expr.Span)
					expectedSig = nil
				}
			} else {
				c.addDiagnostic("invalid_lambda_type", "lambda parameter count does not match expected function type", expr.Span)
				expectedSig = nil
			}
		}
	}
	for i, param := range expr.Parameters {
		paramType := unknownType
		if param.Type != nil {
			paramType = c.resolveDeclaredType(param.Type)
		} else if expectedSig != nil {
			if destructuredTupleArg {
				paramType = params[i]
			} else {
				paramType = expectedSig.Parameters[i]
			}
		} else {
			c.addDiagnostic("invalid_lambda_type", "untyped lambda parameters require a contextual function type", param.Span)
		}
		if destructuredTupleArg {
			if param.Type != nil {
				c.requireAssignable(paramType, params[i], param.Span, "invalid_lambda_type", "lambda parameter does not match expected tuple element type")
			}
			params[i] = paramType
		} else {
			params[i] = paramType
		}
		if param.Name != "_" {
			c.define(param.Name, paramType, false)
		}
	}

	returnType := unknownType
	if expr.Body != nil {
		returnType = c.checkExpr(expr.Body)
		if expectedSig != nil && !containsUnknownType(expectedSig.ReturnType) {
			if !isUnitType(expectedSig.ReturnType) {
				c.requireAssignable(returnType, expectedSig.ReturnType, exprSpan(expr.Body), "invalid_lambda_type", "lambda body does not match expected return type")
			}
			returnType = expectedSig.ReturnType
		}
	}
	if expr.BlockBody != nil {
		expectedReturn := unknownType
		if expectedSig != nil {
			expectedReturn = expectedSig.ReturnType
		}
		c.returnTypes = append(c.returnTypes, expectedReturn)
		implicitReturn := c.checkBlock(expr.BlockBody)
		returnType = c.returnTypes[len(c.returnTypes)-1]
		c.returnTypes = c.returnTypes[:len(c.returnTypes)-1]
		if !isUnknown(implicitReturn) {
			if isUnknown(returnType) {
				returnType = implicitReturn
			} else if !isUnitType(returnType) {
				c.requireAssignable(implicitReturn, returnType, expr.BlockBody.Span, "invalid_lambda_type", "lambda body does not match expected return type")
			}
		}
	}
	if destructuredTupleArg && expectedSig != nil {
		return functionType("<lambda>", Signature{Parameters: expectedSig.Parameters, ReturnType: returnType})
	}
	return functionType("<lambda>", Signature{Parameters: params, ReturnType: returnType})
}

func containsUnknownType(t *Type) bool {
	if t == nil {
		return false
	}
	if isUnknown(t) {
		return true
	}
	for _, arg := range t.Args {
		if containsUnknownType(arg) {
			return true
		}
	}
	if t.Signature != nil {
		for _, param := range t.Signature.Parameters {
			if containsUnknownType(param) {
				return true
			}
		}
		if containsUnknownType(t.Signature.ReturnType) {
			return true
		}
	}
	return false
}

func (c *Checker) checkMemberExpr(expr *parser.MemberExpr) *Type {
	if ident, ok := expr.Receiver.(*parser.Identifier); ok {
		if enumMember, ok := c.tryEnumCaseMemberFromIdentifier(ident.Name, expr.Name); ok {
			return enumMember
		}
		if ident.Name == "Array" && expr.Name == "ofLength" {
			c.addDiagnostic("invalid_member_access", "method 'ofLength' must be called with ()", expr.Span)
			return unknownType
		}
	}
	receiverType := c.checkExpr(expr.Receiver)
	if receiverType.Kind == TypeModule {
		info, ok := c.imports[receiverType.Name]
		if !ok {
			return unknownType
		}
		if fn, ok := info.functions[expr.Name]; ok {
			return functionType(expr.Name, fn)
		}
		if global, ok := info.globals[expr.Name]; ok {
			return global.typ
		}
		if class, ok := info.classes[expr.Name]; ok {
			return &Type{Kind: TypeClass, Name: class.name}
		}
		if object, ok := info.objects[expr.Name]; ok {
			return &Type{Kind: TypeObject, Name: object.name}
		}
		c.addDiagnostic("unknown_member", "unknown imported member '"+expr.Name+"' on module '"+receiverType.Name+"'", expr.Span)
		return unknownType
	}
	if receiverType.Kind == TypeClass {
		if info, ok := c.classes[receiverType.Name]; ok && info.decl.Enum {
			if enumCase, ok := info.enumCases[expr.Name]; ok {
				if len(enumCase.Fields) == 0 {
					return receiverType
				}
				params := make([]*Type, len(enumCase.Fields))
				for i, field := range enumCase.Fields {
					params[i] = c.resolveDeclaredType(field.Type)
				}
				return functionType(expr.Name, Signature{Parameters: params, ReturnType: receiverType})
			}
		}
	}
	if receiverType.Kind == TypeObject {
		if memberType, ok := c.lookupMember(receiverType, expr.Name, expr.Span); ok {
			return memberType
		}
		c.addDiagnostic("unknown_member", "unknown member '"+expr.Name+"'", expr.Span)
		return unknownType
	}
	if memberType, ok := c.lookupMember(receiverType, expr.Name, expr.Span); ok {
		return memberType
	}
	c.addDiagnostic("unknown_member", "unknown member '"+expr.Name+"'", expr.Span)
	return unknownType
}

func (c *Checker) lookupMember(receiver *Type, name string, span parser.Span) (*Type, bool) {
	if isUnknown(receiver) {
		return unknownType, true
	}
	if receiver.Kind == TypeTuple {
		return unknownType, false
	}
	if receiver.Kind == TypeRecord {
		for _, field := range receiver.Fields {
			if field.Name == name {
				return field.Type, true
			}
		}
		return unknownType, false
	}
	if receiver.Kind == TypeBuiltin && receiver.Name == "Array" {
		if name == "size" {
			c.addDiagnostic("invalid_member_access", "method '"+name+"' must be called with ()", span)
			return unknownType, true
		}
		return unknownType, false
	}
	switch receiver.Kind {
	case TypeClass:
		info, ok := c.classes[receiver.Name]
		if !ok {
			return unknownType, false
		}
		subst := c.substForDecl(info.decl.TypeParameters, receiver.Args)
		if field, ok := info.fields[name]; ok {
			if field.decl.Private && !c.canAccessPrivate(info.decl) {
				c.addDiagnostic("private_access", "cannot access private field '"+name+"' outside class '"+info.decl.Name+"'", span)
				return unknownType, true
			}
			return c.instantiateFieldType(field, subst), true
		}
		if methods, ok := info.methods[name]; ok && len(methods) > 0 {
			if hasPrivateOnlyMatch(methods, info.decl, c) {
				c.addDiagnostic("private_access", "cannot access private method '"+name+"' outside class '"+info.decl.Name+"'", span)
				return unknownType, true
			}
			c.addDiagnostic("invalid_member_access", "method '"+name+"' must be called with ()", span)
			return unknownType, true
		}
	case TypeObject:
		info, ok := c.lookupObjectInfo(receiver.Name)
		if !ok {
			return unknownType, false
		}
		subst := c.substForDecl(info.decl.TypeParameters, receiver.Args)
		if field, ok := info.fields[name]; ok {
			if field.decl.Private && !c.canAccessPrivate(info.decl) {
				c.addDiagnostic("private_access", "cannot access private field '"+name+"' outside object '"+info.decl.Name+"'", span)
				return unknownType, true
			}
			return c.instantiateFieldType(field, subst), true
		}
		if methods, ok := info.methods[name]; ok && len(methods) > 0 {
			if hasPrivateOnlyMatch(methods, info.decl, c) {
				c.addDiagnostic("private_access", "cannot access private method '"+name+"' outside object '"+info.decl.Name+"'", span)
				return unknownType, true
			}
			c.addDiagnostic("invalid_member_access", "method '"+name+"' must be called with ()", span)
			return unknownType, true
		}
	case TypeInterface:
		info, ok := c.interfaces[receiver.Name]
		if !ok {
			return unknownType, false
		}
		subst := c.substForDecl(info.decl.TypeParameters, receiver.Args)
		if method, ok := c.lookupInterfaceMethodInfo(info.decl, name, map[string]bool{}); ok {
			_ = c.instantiateInterfaceMethodSignature(method.decl, subst)
			c.addDiagnostic("invalid_member_access", "method '"+name+"' must be called with ()", span)
			return unknownType, true
		}
	}
	return unknownType, false
}

func (c *Checker) checkBinaryOperation(left, right *Type, op string, span parser.Span) *Type {
	switch op {
	case "+":
		if sameType(left, builtin("Str")) || sameType(right, builtin("Str")) {
			return builtin("Str")
		}
		if result, ok := c.resolveOperatorExprType(left, op, []*Type{right}, span); ok {
			return result
		}
		if !isNumeric(left) || !isNumeric(right) {
			c.addDiagnostic("invalid_binary_operand", "operator + requires numeric operands unless one side is Str", span)
			return unknownType
		}
		if !sameType(left, right) {
			c.addDiagnostic("type_mismatch", "arithmetic operands must have the same type", span)
		}
		return left
	case "-", "*", "/", "%":
		if result, ok := c.resolveOperatorExprType(left, op, []*Type{right}, span); ok {
			return result
		}
		if !isNumeric(left) || !isNumeric(right) {
			c.addDiagnostic("invalid_binary_operand", "arithmetic operators require numeric operands", span)
			return unknownType
		}
		if !sameType(left, right) {
			c.addDiagnostic("type_mismatch", "arithmetic operands must have the same type", span)
		}
		return left
	case "&&", "||":
		c.requireAssignable(left, builtin("Bool"), span, "invalid_binary_operand", "logical operators require Bool operands")
		c.requireAssignable(right, builtin("Bool"), span, "invalid_binary_operand", "logical operators require Bool operands")
		return builtin("Bool")
	case "==", "!=":
		if !sameType(left, right) {
			c.addDiagnostic("type_mismatch", "comparison operands must have the same type", span)
		}
		if !c.supportsEquality(left) || !c.supportsEquality(right) {
			c.addDiagnostic("invalid_binary_operand", "equality requires Eq support for this type", span)
		}
		return builtin("Bool")
	case "<", "<=", ">", ">=":
		if !isOrdered(left) || !isOrdered(right) {
			c.addDiagnostic("invalid_binary_operand", "comparison operators require ordered operands", span)
			return builtin("Bool")
		}
		if !sameType(left, right) {
			c.addDiagnostic("type_mismatch", "comparison operands must have the same type", span)
		}
		return builtin("Bool")
	case ":":
		return unknownType
	case ":+":
		if result, ok := c.checkCollectionAppendType(left, right, span); ok {
			return result
		}
		if result, ok := c.resolveOperatorExprType(left, op, []*Type{right}, span); ok {
			return result
		}
		c.addDiagnostic("invalid_binary_operand", "operator :+ requires a collection receiver or matching operator overload", span)
		return unknownType
	case "++":
		if result, ok := c.checkCollectionConcatType(left, right, span); ok {
			return result
		}
		if result, ok := c.resolveOperatorExprType(left, op, []*Type{right}, span); ok {
			return result
		}
		c.addDiagnostic("invalid_binary_operand", "operator ++ requires matching collections or a matching operator overload", span)
		return unknownType
	case ":-", "--", "|", "&", ">>", "<<", "::":
		if result, ok := c.resolveOperatorExprType(left, op, []*Type{right}, span); ok {
			return result
		}
		c.addDiagnostic("invalid_binary_operand", "operator "+op+" requires a matching operator overload", span)
		return unknownType
	default:
		return unknownType
	}
}

func (c *Checker) resolveOperatorExprType(receiver *Type, name string, argTypes []*Type, span parser.Span) (*Type, bool) {
	if isUnknown(receiver) || receiver.Kind != TypeClass {
		return unknownType, false
	}
	class, ok := c.classes[receiver.Name]
	if !ok {
		return unknownType, false
	}
	method, ok := c.findOperatorOverload(class, receiver, name, argTypes, span)
	if !ok {
		return unknownType, false
	}
	subst := c.substForDecl(class.decl.TypeParameters, receiver.Args)
	sig := c.instantiateMethodSignature(method.decl, class.decl, subst)
	return sig.ReturnType, true
}

func (c *Checker) findOperatorOverload(class classInfo, receiver *Type, name string, argTypes []*Type, span parser.Span) (methodInfo, bool) {
	methods, ok := class.methods[name]
	if !ok || len(methods) == 0 {
		return methodInfo{}, false
	}
	subst := c.substForDecl(class.decl.TypeParameters, receiver.Args)
	var matches []methodInfo
	for _, method := range methods {
		if !method.decl.Operator {
			continue
		}
		if method.decl.Private && !c.canAccessPrivate(class.decl) {
			continue
		}
		sig := c.instantiateMethodSignature(method.decl, class.decl, subst)
		if signatureMatches(sig, argTypes) {
			matches = append(matches, method)
		}
	}
	if len(matches) == 1 {
		return matches[0], true
	}
	if len(matches) > 1 {
		c.addDiagnostic("ambiguous_overload", "operator '"+name+"' is ambiguous", span)
	}
	return methodInfo{}, false
}

func (c *Checker) checkCollectionAppendType(left, right *Type, span parser.Span) (*Type, bool) {
	if isUnknown(left) || isUnknown(right) {
		return unknownType, false
	}
	if left.Kind == TypeInterface && left.Name == "List" && len(left.Args) == 1 {
		c.requireAssignable(right, left.Args[0], span, "type_mismatch", "cannot append "+right.String()+" to List["+left.Args[0].String()+"]")
		return left, true
	}
	if left.Kind == TypeInterface && left.Name == "Set" && len(left.Args) == 1 {
		c.requireAssignable(right, left.Args[0], span, "type_mismatch", "cannot add "+right.String()+" to Set["+left.Args[0].String()+"]")
		return left, true
	}
	return unknownType, false
}

func (c *Checker) checkCollectionConcatType(left, right *Type, span parser.Span) (*Type, bool) {
	if isUnknown(left) || isUnknown(right) {
		return unknownType, false
	}
	if left.Kind == TypeInterface && right.Kind == TypeInterface && left.Name == right.Name && len(left.Args) == len(right.Args) {
		switch left.Name {
		case "List", "Set", "Map":
			for i := range left.Args {
				if !sameType(left.Args[i], right.Args[i]) {
					c.addDiagnostic("type_mismatch", "operator ++ requires matching collection element types", span)
					return unknownType, true
				}
			}
			return left, true
		}
	}
	return unknownType, false
}

func (c *Checker) checkAssignmentTarget(target parser.Expr, span parser.Span) (*Type, bool) {
	switch t := target.(type) {
	case *parser.Identifier:
		b, ok := c.lookup(t.Name)
		if !ok {
			c.addDiagnostic("undefined_name", "undefined name '"+t.Name+"'", t.Span)
			return unknownType, false
		}
		if !b.mutable {
			c.addDiagnostic("assign_immutable", "cannot assign to immutable binding '"+t.Name+"'", t.Span)
		}
		return b.typ, b.mutable
	case *parser.MemberExpr:
		receiverType := c.checkExpr(t.Receiver)
		memberType, mutable, ok := c.checkFieldAssignment(t, receiverType)
		if ok {
			return memberType, mutable
		}
		return unknownType, false
	case *parser.IndexExpr:
		elemType := c.checkIndexExpr(t)
		if isUnknown(elemType) {
			return unknownType, false
		}
		return elemType, true
	default:
		c.addDiagnostic("invalid_assignment_target", "invalid assignment target", span)
		return unknownType, false
	}
}

func (c *Checker) allowEqualsAssignment(target parser.Expr) bool {
	member, ok := target.(*parser.MemberExpr)
	if !ok {
		return false
	}
	if c.currentMethod == nil || !c.currentMethod.Constructor {
		return false
	}
	ident, ok := member.Receiver.(*parser.Identifier)
	return ok && ident.Name == "this"
}

func (c *Checker) checkFieldAssignment(expr *parser.MemberExpr, receiverType *Type) (*Type, bool, bool) {
	if isUnknown(receiverType) {
		return unknownType, false, true
	}
	if receiverType.Kind != TypeClass {
		c.addDiagnostic("invalid_assignment_target", "member assignment expects class instance", expr.Span)
		return unknownType, false, true
	}
	info, ok := c.classes[receiverType.Name]
	if !ok {
		c.addDiagnostic("invalid_assignment_target", "member assignment expects class instance", expr.Span)
		return unknownType, false, true
	}
	field, ok := info.fields[expr.Name]
	if !ok {
		if methods, hasMethod := info.methods[expr.Name]; hasMethod && len(methods) > 0 {
			if len(methods) == 1 && methods[0].decl.Private && !c.canAccessPrivate(info.decl) {
				c.addDiagnostic("private_access", "cannot access private method '"+expr.Name+"' outside class '"+info.decl.Name+"'", expr.Span)
				return unknownType, false, true
			}
			c.addDiagnostic("invalid_assignment_target", "cannot assign to method '"+expr.Name+"'", expr.Span)
			return unknownType, false, true
		}
		c.addDiagnostic("unknown_member", "unknown member '"+expr.Name+"'", expr.Span)
		return unknownType, false, true
	}
	if field.decl.Private && !c.canAccessPrivate(info.decl) {
		c.addDiagnostic("private_access", "cannot access private field '"+expr.Name+"' outside class '"+info.decl.Name+"'", expr.Span)
		return unknownType, false, true
	}
	fieldType := c.instantiateFieldType(field, c.substForDecl(info.decl.TypeParameters, receiverType.Args))
	if field.decl.Mutable {
		return fieldType, true, true
	}
	if c.canAssignImmutableField(expr, info.decl) {
		return fieldType, true, true
	}
	c.addDiagnostic("assign_immutable", "cannot assign to immutable field '"+expr.Name+"' outside constructor", expr.Span)
	return fieldType, false, true
}

func (c *Checker) canAssignImmutableField(expr *parser.MemberExpr, owner *parser.ClassDecl) bool {
	if c.currentClass == nil || c.currentMethod == nil || !c.currentMethod.Constructor {
		return false
	}
	if c.currentClass.Name != owner.Name {
		return false
	}
	ident, ok := expr.Receiver.(*parser.Identifier)
	return ok && ident.Name == "this"
}

func (c *Checker) canAccessPrivate(owner *parser.ClassDecl) bool {
	return c.currentClass != nil && c.currentClass.Name == owner.Name
}

func (c *Checker) resolveDeclaredType(ref *parser.TypeRef) *Type {
	typ := c.instantiateTypeRef(ref, nil)
	c.validateTypeRefBounds(ref, typ)
	return typ
}

func (c *Checker) instantiateTypeRef(ref *parser.TypeRef, subst map[string]*Type) *Type {
	if ref == nil {
		return unknownType
	}
	if ref.ReturnType != nil {
		params := make([]*Type, len(ref.ParameterTypes))
		for i, param := range ref.ParameterTypes {
			params[i] = c.instantiateTypeRef(param, subst)
		}
		return &Type{
			Kind: TypeFunction,
			Name: "func",
			Signature: &Signature{
				Parameters: params,
				ReturnType: c.instantiateTypeRef(ref.ReturnType, subst),
			},
		}
	}
	if len(ref.TupleElements) > 0 {
		args := make([]*Type, len(ref.TupleElements))
		for i, arg := range ref.TupleElements {
			args[i] = c.instantiateTypeRef(arg, subst)
		}
		return &Type{Kind: TypeTuple, Name: "Tuple", Args: args, TupleNames: append([]string(nil), ref.TupleNames...)}
	}
	if len(ref.RecordFields) > 0 {
		fields := make([]RecordField, len(ref.RecordFields))
		for i, field := range ref.RecordFields {
			fields[i] = RecordField{Name: field.Name, Type: c.instantiateTypeRef(field.Type, subst)}
		}
		return &Type{Kind: TypeRecord, Name: "Record", Fields: fields}
	}
	if subst != nil {
		if resolved, ok := subst[ref.Name]; ok && len(ref.Arguments) == 0 {
			return resolved
		}
	}
	args := make([]*Type, len(ref.Arguments))
	for i, arg := range ref.Arguments {
		args[i] = c.instantiateTypeRef(arg, subst)
	}
	kind := c.kindOf(ref.Name)
	if kind == "" {
		kind = TypeUnknown
	}
	name := ref.Name
	if class, ok := c.importedClasses[ref.Name]; ok {
		name = class.name
	}
	if qualified, ok := c.importedInterfaceNames[ref.Name]; ok {
		name = qualified
	}
	return &Type{Kind: kind, Name: name, Args: args}
}

func (c *Checker) validateTypeParameterBounds(params []parser.TypeParameter) {
	for _, param := range params {
		for _, bound := range param.Bounds {
			boundType := c.resolveDeclaredType(bound)
			if boundType.Kind != TypeInterface {
				c.addDiagnostic("invalid_type_bound", "type parameter '"+param.Name+"' bound must be an interface", bound.Span)
			}
		}
	}
}

func (c *Checker) validateTypeRefBounds(ref *parser.TypeRef, typ *Type) {
	if ref == nil || typ == nil || len(ref.Arguments) == 0 {
		return
	}
	params, ok := c.typeParametersForName(ref.Name)
	if !ok || len(params) == 0 || len(params) != len(typ.Args) {
		return
	}
	c.checkTypeArgBounds(typ.Args, params, ref.Span)
}

func (c *Checker) instantiateMethodSignature(method *parser.MethodDecl, owner *parser.ClassDecl, subst map[string]*Type) Signature {
	effective := mergeSubst(subst, c.substForDecl(owner.TypeParameters, nil))
	effective = mergeSubst(effective, c.substForDecl(method.TypeParameters, nil))
	params := make([]*Type, len(method.Parameters))
	for i, param := range method.Parameters {
		params[i] = c.instantiateTypeRef(param.Type, effective)
	}
	result := unknownType
	if !method.Constructor {
		result = c.instantiateTypeRef(method.ReturnType, effective)
	}
	return Signature{Parameters: params, ReturnType: result, Variadic: len(method.Parameters) > 0 && method.Parameters[len(method.Parameters)-1].Variadic}
}

func (c *Checker) instantiateFunctionSignature(fn *parser.FunctionDecl, subst map[string]*Type) Signature {
	effective := mergeSubst(subst, c.substForDecl(fn.TypeParameters, nil))
	params := make([]*Type, len(fn.Parameters))
	for i, param := range fn.Parameters {
		params[i] = c.instantiateTypeRef(param.Type, effective)
	}
	return Signature{
		Parameters: params,
		ReturnType: c.instantiateTypeRef(fn.ReturnType, effective),
		Variadic:   len(fn.Parameters) > 0 && fn.Parameters[len(fn.Parameters)-1].Variadic,
	}
}

func (c *Checker) checkConstructorRules(class classInfo) {
	if len(class.constructors) == 0 {
		if missing := c.uninitializedLetFields(class.decl, nil); len(missing) > 0 {
			c.addDiagnostic("constructor_required", "class '"+class.decl.Name+"' requires a constructor to initialize immutable fields: "+joinNames(missing), class.decl.Span)
		}
		return
	}
	seen := map[string]*parser.MethodDecl{}
	for _, ctor := range class.constructors {
		key := methodSignatureKey(ctor)
		if prev, ok := seen[key]; ok {
			c.addDiagnostic("duplicate_constructor", "duplicate constructor overload for class '"+class.decl.Name+"'", ctor.Span)
			c.addDiagnostic("duplicate_constructor", "duplicate constructor overload for class '"+class.decl.Name+"'", prev.Span)
			continue
		}
		seen[key] = ctor
		if missing := c.uninitializedLetFields(class.decl, ctor); len(missing) > 0 {
			c.addDiagnostic("uninitialized_field", "constructor 'init' must initialize immutable fields: "+joinNames(missing), ctor.Span)
		}
	}
}

func (c *Checker) uninitializedLetFields(owner *parser.ClassDecl, ctor *parser.MethodDecl) []string {
	initialized := map[string]bool{}
	if ctor != nil {
		c.collectInitializedFields(ctor.Body, initialized)
	}
	var missing []string
	for _, field := range owner.Fields {
		if field.Mutable {
			continue
		}
		if constructorVisibleField(field) {
			continue
		}
		if field.Initializer != nil {
			continue
		}
		if !initialized[field.Name] {
			missing = append(missing, field.Name)
		}
	}
	return missing
}

func (c *Checker) collectInitializedFields(block *parser.BlockStmt, initialized map[string]bool) {
	if block == nil {
		return
	}
	for _, stmt := range block.Statements {
		switch s := stmt.(type) {
		case *parser.AssignmentStmt:
			if s.Operator != "=" {
				continue
			}
			member, ok := s.Target.(*parser.MemberExpr)
			if !ok {
				continue
			}
			ident, ok := member.Receiver.(*parser.Identifier)
			if ok && ident.Name == "this" {
				initialized[member.Name] = true
			}
		case *parser.IfStmt:
			// Keep constructor rules simple: writes must happen unconditionally in the constructor body.
		}
	}
}

func (c *Checker) checkArgTypes(args []parser.Expr) []*Type {
	result := make([]*Type, len(args))
	for i, arg := range args {
		result[i] = c.checkExpr(arg)
	}
	return result
}

func constructorVisibleField(field parser.FieldDecl) bool {
	return !field.Private && field.Initializer == nil
}

func (c *Checker) currentFieldType(name string) (*Type, bool) {
	if c.currentClass == nil {
		return nil, false
	}
	for _, field := range c.currentClass.Fields {
		if field.Name == name {
			return c.classFieldType(c.currentClass, field), true
		}
	}
	return nil, false
}

func (c *Checker) classFieldType(owner *parser.ClassDecl, field parser.FieldDecl) *Type {
	info, ok := c.classes[owner.Name]
	if !ok && owner.Object {
		info, ok = c.objects[owner.Name]
	}
	if ok {
		if known, ok := info.fields[field.Name]; ok && known.typ != nil {
			return known.typ
		}
	}
	if field.Type == nil {
		return unknownType
	}
	return c.resolveDeclaredType(field.Type)
}

func (c *Checker) instantiateFieldType(field fieldInfo, subst map[string]*Type) *Type {
	if field.typ != nil {
		return field.typ
	}
	if field.decl.Type == nil {
		return unknownType
	}
	return c.instantiateTypeRef(field.decl.Type, subst)
}

func primaryConstructorParams(class *parser.ClassDecl) []parser.Parameter {
	if !hasSafeImplicitConstructor(class) {
		return nil
	}
	params := make([]parser.Parameter, 0, len(class.Fields))
	for _, field := range class.Fields {
		if constructorVisibleField(field) {
			params = append(params, parser.Parameter{Name: field.Name, Type: field.Type, Span: field.Span})
		}
	}
	return params
}

func (c *Checker) primaryConstructorSignature(class *parser.ClassDecl) Signature {
	params := primaryConstructorParams(class)
	out := make([]*Type, len(params))
	for i, param := range params {
		out[i] = c.resolveDeclaredType(param.Type)
	}
	return Signature{Parameters: out, ReturnType: &Type{Kind: TypeClass, Name: class.Name}}
}

func hasSafeImplicitConstructor(class *parser.ClassDecl) bool {
	for _, field := range class.Fields {
		if field.Mutable {
			continue
		}
		if constructorVisibleField(field) {
			continue
		}
		if field.Initializer != nil {
			continue
		}
		return false
	}
	return true
}

func callArgValues(args []parser.CallArg) []parser.Expr {
	values := make([]parser.Expr, len(args))
	for i, arg := range args {
		values[i] = arg.Value
	}
	return values
}

func hasNamedCallArgs(args []parser.CallArg) bool {
	for _, arg := range args {
		if arg.Name != "" {
			return true
		}
	}
	return false
}

func tryReorderCallArgs(params []parser.Parameter, args []parser.CallArg) ([]parser.Expr, bool) {
	if len(args) == 0 {
		if len(params) == 0 {
			return []parser.Expr{}, true
		}
		return nil, false
	}
	if len(params) > 0 && params[len(params)-1].Variadic {
		return nil, false
	}
	ordered := make([]parser.Expr, len(params))
	filled := make([]bool, len(params))
	seenNamed := false
	pos := 0
	for _, arg := range args {
		if arg.Name == "" {
			if seenNamed || pos >= len(params) {
				return nil, false
			}
			ordered[pos] = arg.Value
			filled[pos] = true
			pos++
			continue
		}
		seenNamed = true
		paramIndex := -1
		for i, param := range params {
			if param.Name == arg.Name {
				paramIndex = i
				break
			}
		}
		if paramIndex < 0 || filled[paramIndex] {
			return nil, false
		}
		ordered[paramIndex] = arg.Value
		filled[paramIndex] = true
	}
	for _, ok := range filled {
		if !ok {
			return nil, false
		}
	}
	return ordered, true
}

func (c *Checker) reorderCallArgs(params []parser.Parameter, args []parser.CallArg, span parser.Span, callable string) ([]parser.Expr, bool) {
	if len(params) > 0 && params[len(params)-1].Variadic {
		c.addDiagnostic("invalid_named_argument", "named arguments are not supported for variadic "+callable, span)
		return nil, false
	}
	ordered := make([]parser.Expr, len(params))
	filled := make([]bool, len(params))
	seenNamed := false
	pos := 0
	for _, arg := range args {
		if arg.Name == "" {
			if seenNamed {
				c.addDiagnostic("invalid_named_argument", "positional arguments cannot follow named arguments", arg.Span)
				return nil, false
			}
			if pos >= len(params) {
				c.addDiagnostic("invalid_argument_count", fmt.Sprintf("%s expects %d arguments, got %d", callable, len(params), len(args)), span)
				return nil, false
			}
			ordered[pos] = arg.Value
			filled[pos] = true
			pos++
			continue
		}
		seenNamed = true
		paramIndex := -1
		for i, param := range params {
			if param.Name == arg.Name {
				paramIndex = i
				break
			}
		}
		if paramIndex < 0 {
			c.addDiagnostic("unknown_argument", "unknown named argument '"+arg.Name+"'", arg.Span)
			return nil, false
		}
		if filled[paramIndex] {
			c.addDiagnostic("duplicate_argument", "argument '"+arg.Name+"' was provided more than once", arg.Span)
			return nil, false
		}
		ordered[paramIndex] = arg.Value
		filled[paramIndex] = true
	}
	for i, ok := range filled {
		if !ok {
			c.addDiagnostic("invalid_argument_count", fmt.Sprintf("missing argument '%s' in %s", params[i].Name, callable), span)
			return nil, false
		}
	}
	return ordered, true
}

func (c *Checker) resolveConstructorOverload(class classInfo, argTypes []*Type, span parser.Span) (*parser.MethodDecl, bool) {
	candidates := make([]*parser.MethodDecl, 0, len(class.constructors))
	for _, ctor := range class.constructors {
		sig := c.instantiateMethodSignature(ctor, class.decl, nil)
		if signatureMatches(sig, argTypes) {
			candidates = append(candidates, ctor)
		}
	}
	primarySig := c.primaryConstructorSignature(class.decl)
	primaryMatches := signatureMatches(primarySig, argTypes)
	if primaryMatches && len(candidates) == 0 {
		return nil, true
	}
	if primaryMatches && len(candidates) > 0 {
		if len(candidates) == 1 {
			return candidates[0], true
		}
		c.addDiagnostic("ambiguous_overload", "constructor call for class '"+class.decl.Name+"' is ambiguous", span)
		return nil, false
	}
	if len(candidates) == 1 {
		return candidates[0], true
	}
	if len(candidates) > 1 {
		c.addDiagnostic("ambiguous_overload", "constructor call for class '"+class.decl.Name+"' is ambiguous", span)
		return nil, false
	}
	c.addDiagnostic("no_matching_overload", fmt.Sprintf("no constructor overload for class '%s' matches %d arguments", class.decl.Name, len(argTypes)), span)
	return nil, false
}

func (c *Checker) tryResolveMethodOverload(class classInfo, receiver *Type, name string, argTypes []*Type) (methodInfo, bool) {
	methods, ok := class.methods[name]
	if !ok || len(methods) == 0 {
		return methodInfo{}, false
	}
	subst := c.substForDecl(class.decl.TypeParameters, receiver.Args)
	var matches []methodInfo
	for _, method := range methods {
		if method.decl.Private && !c.canAccessPrivate(class.decl) {
			continue
		}
		sig := c.instantiateMethodSignature(method.decl, class.decl, subst)
		if len(method.decl.TypeParameters) > 0 {
			inferred, ok := c.inferCallableTypeArgs(method.decl.TypeParameters, method.decl.Parameters, argTypes, subst)
			if ok {
				sig = c.instantiateMethodSignature(method.decl, class.decl, mergeSubst(inferred, subst))
			}
		}
		if signatureMatches(sig, argTypes) {
			matches = append(matches, method)
		}
	}
	if len(matches) == 1 {
		return matches[0], true
	}
	return methodInfo{}, false
}

func (c *Checker) resolveMethodOverload(class classInfo, receiver *Type, name string, argTypes []*Type, span parser.Span) (methodInfo, bool) {
	methods, ok := class.methods[name]
	if !ok || len(methods) == 0 {
		c.addDiagnostic("unknown_member", "unknown member '"+name+"'", span)
		return methodInfo{}, false
	}
	if method, ok := c.tryResolveMethodOverload(class, receiver, name, argTypes); ok {
		return method, true
	}
	subst := c.substForDecl(class.decl.TypeParameters, receiver.Args)
	matchCount := 0
	for _, method := range methods {
		if method.decl.Private && !c.canAccessPrivate(class.decl) {
			continue
		}
		sig := c.instantiateMethodSignature(method.decl, class.decl, subst)
		if len(method.decl.TypeParameters) > 0 {
			inferred, ok := c.inferCallableTypeArgs(method.decl.TypeParameters, method.decl.Parameters, argTypes, subst)
			if ok {
				sig = c.instantiateMethodSignature(method.decl, class.decl, mergeSubst(inferred, subst))
			}
		}
		if signatureMatches(sig, argTypes) {
			matchCount++
		}
	}
	if matchCount > 1 {
		c.addDiagnostic("ambiguous_overload", "method call '"+name+"' is ambiguous", span)
		return methodInfo{}, false
	}
	if hasPrivateOnlyMatch(methods, class.decl, c) {
		c.addDiagnostic("private_access", "cannot access private method '"+name+"' outside class '"+class.decl.Name+"'", span)
		return methodInfo{}, false
	}
	c.addDiagnostic("no_matching_overload", fmt.Sprintf("no overload of method '%s' matches %d arguments", name, len(argTypes)), span)
	return methodInfo{}, false
}

func (c *Checker) resolveNamedConstructorOverload(class classInfo, args []parser.CallArg, span parser.Span) (*parser.MethodDecl, []parser.Expr, bool) {
	var (
		matchCtor  *parser.MethodDecl
		matchArgs  []parser.Expr
		matchCount int
	)
	for _, ctor := range class.constructors {
		ordered, ok := tryReorderCallArgs(ctor.Parameters, args)
		if !ok {
			continue
		}
		argTypes := c.checkArgTypes(ordered)
		sig := c.instantiateMethodSignature(ctor, class.decl, nil)
		if !signatureMatches(sig, argTypes) {
			continue
		}
		matchCtor = ctor
		matchArgs = ordered
		matchCount++
	}
	primaryParams := primaryConstructorParams(class.decl)
	if ordered, ok := tryReorderCallArgs(primaryParams, args); ok {
		argTypes := c.checkArgTypes(ordered)
		if signatureMatches(c.primaryConstructorSignature(class.decl), argTypes) {
			matchArgs = ordered
			matchCount++
		}
	}
	if matchCount == 1 {
		return matchCtor, matchArgs, true
	}
	if matchCount == 2 && matchCtor != nil {
		return matchCtor, matchArgs, true
	}
	if matchCount > 1 {
		c.addDiagnostic("ambiguous_overload", "constructor call for class '"+class.decl.Name+"' is ambiguous", span)
		return nil, nil, false
	}
	if len(class.constructors) == 1 {
		c.reorderCallArgs(class.constructors[0].Parameters, args, span, "constructor '"+class.decl.Name+"'")
		return nil, nil, false
	}
	if len(class.constructors) == 0 {
		c.reorderCallArgs(primaryParams, args, span, "constructor '"+class.decl.Name+"'")
		return nil, nil, false
	}
	c.addDiagnostic("no_matching_overload", fmt.Sprintf("no constructor overload for class '%s' matches %d arguments", class.decl.Name, len(args)), span)
	return nil, nil, false
}

func (c *Checker) tryResolveNamedMethodOverload(class classInfo, receiver *Type, name string, args []parser.CallArg) (methodInfo, []parser.Expr, bool) {
	methods, ok := class.methods[name]
	if !ok || len(methods) == 0 {
		return methodInfo{}, nil, false
	}
	subst := c.substForDecl(class.decl.TypeParameters, receiver.Args)
	type candidate struct {
		method methodInfo
		args   []parser.Expr
	}
	var matches []candidate
	for _, method := range methods {
		if method.decl.Private && !c.canAccessPrivate(class.decl) {
			continue
		}
		ordered, ok := tryReorderCallArgs(method.decl.Parameters, args)
		if !ok {
			continue
		}
		argTypes := c.checkArgTypes(ordered)
		sig := c.instantiateMethodSignature(method.decl, class.decl, subst)
		if len(method.decl.TypeParameters) > 0 {
			inferred, ok := c.inferCallableTypeArgs(method.decl.TypeParameters, method.decl.Parameters, argTypes, subst)
			if ok {
				sig = c.instantiateMethodSignature(method.decl, class.decl, mergeSubst(inferred, subst))
			}
		}
		if signatureMatches(sig, argTypes) {
			matches = append(matches, candidate{method: method, args: ordered})
		}
	}
	if len(matches) == 1 {
		return matches[0].method, matches[0].args, true
	}
	return methodInfo{}, nil, false
}

func (c *Checker) resolveNamedMethodOverload(class classInfo, receiver *Type, name string, args []parser.CallArg, span parser.Span) (methodInfo, []parser.Expr, bool) {
	methods, ok := class.methods[name]
	if !ok || len(methods) == 0 {
		c.addDiagnostic("unknown_member", "unknown member '"+name+"'", span)
		return methodInfo{}, nil, false
	}
	if method, ordered, ok := c.tryResolveNamedMethodOverload(class, receiver, name, args); ok {
		return method, ordered, true
	}
	subst := c.substForDecl(class.decl.TypeParameters, receiver.Args)
	type candidate struct {
		method methodInfo
		args   []parser.Expr
	}
	var matches []candidate
	for _, method := range methods {
		if method.decl.Private && !c.canAccessPrivate(class.decl) {
			continue
		}
		ordered, ok := tryReorderCallArgs(method.decl.Parameters, args)
		if !ok {
			continue
		}
		argTypes := c.checkArgTypes(ordered)
		sig := c.instantiateMethodSignature(method.decl, class.decl, subst)
		if len(method.decl.TypeParameters) > 0 {
			inferred, ok := c.inferCallableTypeArgs(method.decl.TypeParameters, method.decl.Parameters, argTypes, subst)
			if ok {
				sig = c.instantiateMethodSignature(method.decl, class.decl, mergeSubst(inferred, subst))
			}
		}
		if signatureMatches(sig, argTypes) {
			matches = append(matches, candidate{method: method, args: ordered})
		}
	}
	if len(matches) == 1 {
		return matches[0].method, matches[0].args, true
	}
	if len(matches) > 1 {
		c.addDiagnostic("ambiguous_overload", "method call '"+name+"' is ambiguous", span)
		return methodInfo{}, nil, false
	}
	if hasPrivateOnlyMatch(methods, class.decl, c) {
		c.addDiagnostic("private_access", "cannot access private method '"+name+"' outside class '"+class.decl.Name+"'", span)
		return methodInfo{}, nil, false
	}
	if len(methods) == 1 {
		c.reorderCallArgs(methods[0].decl.Parameters, args, span, "method '"+name+"'")
		return methodInfo{}, nil, false
	}
	c.addDiagnostic("no_matching_overload", fmt.Sprintf("no overload of method '%s' matches %d arguments", name, len(args)), span)
	return methodInfo{}, nil, false
}

func (c *Checker) findMatchingMethodOverload(class classInfo, name string, paramTypes []*Type) (methodInfo, bool) {
	methods, ok := class.methods[name]
	if !ok {
		return methodInfo{}, false
	}
	for _, method := range methods {
		sig := c.instantiateMethodSignature(method.decl, class.decl, nil)
		if signatureMatches(sig, paramTypes) {
			return method, true
		}
	}
	return methodInfo{}, false
}

func signatureMatches(sig Signature, argTypes []*Type) bool {
	if !validArgCount(sig, len(argTypes)) {
		return false
	}
	for i := range argTypes {
		expected, ok := paramTypeForArg(sig, i)
		if !ok || !sameType(expected, argTypes[i]) {
			return false
		}
	}
	return true
}

func methodSignatureKey(method *parser.MethodDecl) string {
	sig := method.Name + "("
	for i, param := range method.Parameters {
		if i > 0 {
			sig += ","
		}
		sig += param.Type.Name
		if param.Variadic {
			sig += "..."
		}
		for _, arg := range param.Type.Arguments {
			sig += "[" + arg.Name + "]"
		}
	}
	return sig + ")"
}

func validArgCount(sig Signature, count int) bool {
	if sig.Variadic {
		return count >= len(sig.Parameters)-1
	}
	return count == len(sig.Parameters)
}

func expectedArgCount(sig Signature) string {
	if sig.Variadic {
		return fmt.Sprintf("at least %d", len(sig.Parameters)-1)
	}
	return fmt.Sprintf("%d", len(sig.Parameters))
}

func paramTypeForArg(sig Signature, index int) (*Type, bool) {
	if !sig.Variadic {
		if index < len(sig.Parameters) {
			return sig.Parameters[index], true
		}
		return nil, false
	}
	if len(sig.Parameters) == 0 {
		return nil, false
	}
	last := len(sig.Parameters) - 1
	if index < last {
		return sig.Parameters[index], true
	}
	return sig.Parameters[last], true
}

func joinNames(names []string) string {
	result := ""
	for i, name := range names {
		if i > 0 {
			result += ", "
		}
		result += name
	}
	return result
}

func hasPrivateOnlyMatch(methods []methodInfo, owner *parser.ClassDecl, c *Checker) bool {
	if c.canAccessPrivate(owner) {
		return false
	}
	for _, method := range methods {
		if method.decl.Private {
			return true
		}
	}
	return false
}

func (c *Checker) instantiateInterfaceMethodSignature(method parser.InterfaceMethod, subst map[string]*Type) Signature {
	effective := mergeSubst(subst, c.substForDecl(method.TypeParameters, nil))
	params := make([]*Type, len(method.Parameters))
	for i, param := range method.Parameters {
		params[i] = c.instantiateTypeRef(param.Type, effective)
	}
	return Signature{
		Parameters: params,
		ReturnType: c.instantiateTypeRef(method.ReturnType, effective),
		Variadic:   len(method.Parameters) > 0 && method.Parameters[len(method.Parameters)-1].Variadic,
	}
}

func (c *Checker) resolveInterfaceMethodCallSignature(info interfaceInfo, receiver *Type, method parser.InterfaceMethod, args []parser.Expr, span parser.Span) (Signature, bool) {
	baseSubst := c.substForDecl(info.decl.TypeParameters, receiver.Args)
	sig := c.instantiateInterfaceMethodSignature(method, baseSubst)
	if len(method.TypeParameters) == 0 {
		if !validArgCount(sig, len(args)) {
			c.addDiagnostic("invalid_argument_count", fmt.Sprintf("method '%s' expects %s arguments, got %d", method.Name, expectedArgCount(sig), len(args)), span)
			return Signature{}, false
		}
		return sig, true
	}
	inferred, ok := c.inferCallableTypeArgsFromExprs(method.TypeParameters, method.Parameters, args, baseSubst)
	if !ok {
		c.addDiagnostic("cannot_infer_type_args", "cannot infer type arguments for method '"+method.Name+"'", span)
		return Signature{}, false
	}
	if !c.checkTypeArgBounds(c.typeArgsInOrder(method.TypeParameters, inferred), method.TypeParameters, span) {
		return Signature{}, false
	}
	sig = c.instantiateInterfaceMethodSignature(method, mergeSubst(inferred, baseSubst))
	if !validArgCount(sig, len(args)) {
		c.addDiagnostic("invalid_argument_count", fmt.Sprintf("method '%s' expects %s arguments, got %d", method.Name, expectedArgCount(sig), len(args)), span)
		return Signature{}, false
	}
	return sig, true
}

func (c *Checker) tryResolveDefaultInterfaceMethod(receiver *Type, class classInfo, name string, args []parser.Expr, span parser.Span) (Signature, bool) {
	seen := map[string]bool{}
	for _, impl := range class.decl.Implements {
		if sig, ok := c.tryResolveDefaultInterfaceMethodInRef(receiver, impl, name, args, seen, span); ok {
			return sig, true
		}
	}
	return Signature{}, false
}

func (c *Checker) classHasDefaultInterfaceMethod(class classInfo, name string) bool {
	seen := map[string]bool{}
	for _, impl := range class.decl.Implements {
		if c.interfaceRefHasDefaultMethod(impl, name, seen) {
			return true
		}
	}
	return false
}

func (c *Checker) tryResolveNamedDefaultInterfaceMethod(receiver *Type, class classInfo, name string, args []parser.CallArg, span parser.Span) (Signature, []parser.Expr, bool) {
	seen := map[string]bool{}
	for _, impl := range class.decl.Implements {
		if sig, ordered, ok := c.tryResolveNamedDefaultInterfaceMethodInRef(receiver, impl, name, args, seen, span); ok {
			return sig, ordered, true
		}
	}
	return Signature{}, nil, false
}

func (c *Checker) interfaceRefHasDefaultMethod(ref *parser.TypeRef, name string, seen map[string]bool) bool {
	if ref == nil || seen[ref.Name] {
		return false
	}
	seen[ref.Name] = true
	info, ok := c.lookupAnyInterfaceInfo(ref.Name)
	if !ok {
		return false
	}
	for _, method := range info.decl.Methods {
		if method.Name == name && method.Body != nil {
			return true
		}
	}
	for _, parent := range info.decl.Extends {
		if c.interfaceRefHasDefaultMethod(parent, name, seen) {
			return true
		}
	}
	return false
}

func (c *Checker) tryResolveDefaultInterfaceMethodInRef(receiver *Type, ref *parser.TypeRef, name string, args []parser.Expr, seen map[string]bool, span parser.Span) (Signature, bool) {
	if ref == nil || seen[ref.Name] {
		return Signature{}, false
	}
	seen[ref.Name] = true
	info, ok := c.lookupAnyInterfaceInfo(ref.Name)
	if !ok {
		return Signature{}, false
	}
	classSubst := c.substForDecl(c.classTypeParameters(receiver.Name), receiver.Args)
	ifaceType := c.instantiateTypeRef(ref, classSubst)
	for _, method := range info.decl.Methods {
		if method.Name != name || method.Body == nil {
			continue
		}
		if sig, ok := c.tryResolveInterfaceMethodCallSignature(info, ifaceType, method, args); ok {
			return sig, true
		}
	}
	for _, parent := range info.decl.Extends {
		if sig, ok := c.tryResolveDefaultInterfaceMethodInRef(receiver, parent, name, args, seen, span); ok {
			return sig, true
		}
	}
	return Signature{}, false
}

func (c *Checker) tryResolveNamedDefaultInterfaceMethodInRef(receiver *Type, ref *parser.TypeRef, name string, args []parser.CallArg, seen map[string]bool, span parser.Span) (Signature, []parser.Expr, bool) {
	if ref == nil || seen[ref.Name] {
		return Signature{}, nil, false
	}
	seen[ref.Name] = true
	info, ok := c.lookupAnyInterfaceInfo(ref.Name)
	if !ok {
		return Signature{}, nil, false
	}
	classSubst := c.substForDecl(c.classTypeParameters(receiver.Name), receiver.Args)
	ifaceType := c.instantiateTypeRef(ref, classSubst)
	for _, method := range info.decl.Methods {
		if method.Name != name || method.Body == nil || (len(method.Parameters) > 0 && method.Parameters[len(method.Parameters)-1].Variadic) {
			continue
		}
		ordered, ok := tryReorderCallArgs(method.Parameters, args)
		if !ok {
			continue
		}
		if sig, ok := c.tryResolveInterfaceMethodCallSignature(info, ifaceType, method, ordered); ok {
			return sig, ordered, true
		}
	}
	for _, parent := range info.decl.Extends {
		if sig, ordered, ok := c.tryResolveNamedDefaultInterfaceMethodInRef(receiver, parent, name, args, seen, span); ok {
			return sig, ordered, true
		}
	}
	return Signature{}, nil, false
}

func (c *Checker) classTypeParameters(name string) []parser.TypeParameter {
	if info, ok := c.lookupClassInfo(name); ok {
		return info.decl.TypeParameters
	}
	return nil
}

func (c *Checker) lookupAnyInterfaceInfo(name string) (interfaceInfo, bool) {
	if info, ok := c.interfaces[name]; ok {
		return info, true
	}
	if info, ok := c.importedInterfaces[name]; ok {
		return info, true
	}
	return interfaceInfo{}, false
}

func (c *Checker) tryResolveInterfaceMethodCallSignature(info interfaceInfo, receiver *Type, method parser.InterfaceMethod, args []parser.Expr) (Signature, bool) {
	baseSubst := c.substForDecl(info.decl.TypeParameters, receiver.Args)
	sig := c.instantiateInterfaceMethodSignature(method, baseSubst)
	if len(method.TypeParameters) == 0 {
		if !validArgCount(sig, len(args)) {
			return Signature{}, false
		}
		return sig, true
	}
	inferred, ok := c.inferCallableTypeArgsFromExprs(method.TypeParameters, method.Parameters, args, baseSubst)
	if !ok {
		return Signature{}, false
	}
	sig = c.instantiateInterfaceMethodSignature(method, mergeSubst(inferred, baseSubst))
	if !validArgCount(sig, len(args)) {
		return Signature{}, false
	}
	return sig, true
}

func (c *Checker) substForDecl(params []parser.TypeParameter, args []*Type) map[string]*Type {
	if len(params) == 0 {
		return nil
	}
	result := map[string]*Type{}
	for i, param := range params {
		if i < len(args) && args[i] != nil {
			result[param.Name] = args[i]
		} else {
			result[param.Name] = &Type{Kind: TypeParam, Name: param.Name}
		}
	}
	return result
}

func mergeSubst(primary, fallback map[string]*Type) map[string]*Type {
	if primary == nil && fallback == nil {
		return nil
	}
	result := map[string]*Type{}
	for k, v := range fallback {
		result[k] = v
	}
	for k, v := range primary {
		result[k] = v
	}
	return result
}

func constructorTypeArgs(owner *parser.ClassDecl, callee parser.Expr) map[string]*Type {
	_ = callee
	if len(owner.TypeParameters) == 0 {
		return nil
	}
	result := map[string]*Type{}
	for _, param := range owner.TypeParameters {
		result[param.Name] = &Type{Kind: TypeParam, Name: param.Name}
	}
	return result
}

func (c *Checker) lookupTypeInstance(name string) (*Type, bool) {
	if _, ok := c.classes[name]; ok {
		return &Type{Kind: TypeClass, Name: name}, true
	}
	if class, ok := c.importedClasses[name]; ok {
		return &Type{Kind: TypeClass, Name: class.name}, true
	}
	if _, ok := c.interfaces[name]; ok {
		return &Type{Kind: TypeInterface, Name: name}, true
	}
	if _, ok := c.importedInterfaces[name]; ok {
		return &Type{Kind: TypeInterface, Name: c.importedInterfaceNames[name]}, true
	}
	return nil, false
}

func (c *Checker) lookupObjectInfo(name string) (classInfo, bool) {
	if info, ok := c.objects[name]; ok {
		return info, true
	}
	if info, ok := c.importedObjects[name]; ok {
		return info, true
	}
	return classInfo{}, false
}

func (c *Checker) callNameShadowed(name string) bool {
	if _, _, ok := c.lookupWithDepth(name); ok {
		return true
	}
	if _, ok := c.globals[name]; ok {
		return true
	}
	if _, ok := c.functions[name]; ok {
		return true
	}
	if _, ok := c.imports[name]; ok {
		return true
	}
	if _, ok := c.objects[name]; ok {
		return true
	}
	if _, ok := c.classes[name]; ok {
		return true
	}
	if _, ok := c.importedObjects[name]; ok {
		return true
	}
	if _, ok := c.importedClasses[name]; ok {
		return true
	}
	return false
}

func (c *Checker) identifierShadowsTypeName(name string) bool {
	if _, _, ok := c.lookupWithDepth(name); ok {
		return true
	}
	if _, ok := c.currentFieldType(name); ok {
		return true
	}
	if binding, ok := c.globals[name]; ok && binding.typ != nil && binding.typ.Kind != TypeObject {
		return true
	}
	if _, ok := c.functions[name]; ok {
		return true
	}
	if _, ok := c.imports[name]; ok {
		return true
	}
	if _, ok := c.importedObjects[name]; ok {
		return true
	}
	return isBuiltinValue(name)
}

func (c *Checker) tryEnumCaseMemberFromIdentifier(typeName, memberName string) (*Type, bool) {
	info, ok := c.lookupClassInfo(typeName)
	if !ok || !info.decl.Enum {
		return nil, false
	}
	enumCase, ok := info.enumCases[memberName]
	if !ok {
		return nil, false
	}
	receiverType := &Type{Kind: TypeClass, Name: info.name}
	if len(enumCase.Fields) == 0 {
		return receiverType, true
	}
	params := make([]*Type, len(enumCase.Fields))
	for i, field := range enumCase.Fields {
		params[i] = c.resolveDeclaredType(field.Type)
	}
	return functionType(memberName, Signature{Parameters: params, ReturnType: receiverType}), true
}

func (c *Checker) tryEnumCaseCallFromIdentifier(typeName, caseName string, args []parser.CallArg, span parser.Span) (*Type, bool) {
	info, ok := c.lookupClassInfo(typeName)
	if !ok || !info.decl.Enum {
		return nil, false
	}
	enumCase, ok := info.enumCases[caseName]
	if !ok {
		return nil, false
	}
	params := make([]parser.Parameter, len(enumCase.Fields))
	sig := Signature{Parameters: make([]*Type, len(enumCase.Fields)), ReturnType: &Type{Kind: TypeClass, Name: info.name}}
	for i, field := range enumCase.Fields {
		params[i] = parser.Parameter{Name: field.Name, Type: field.Type, Span: field.Span}
		sig.Parameters[i] = c.resolveDeclaredType(field.Type)
	}
	orderedArgs := callArgValues(args)
	if hasNamedCallArgs(args) {
		reordered, ok := c.reorderCallArgs(params, args, span, "enum case '"+caseName+"'")
		if !ok {
			c.checkArgTypes(callArgValues(args))
			return sig.ReturnType, true
		}
		orderedArgs = reordered
	}
	if !validArgCount(sig, len(orderedArgs)) {
		c.addDiagnostic("invalid_argument_count", fmt.Sprintf("enum case '%s' expects %s arguments, got %d", caseName, expectedArgCount(sig), len(orderedArgs)), span)
	}
	c.checkCallArgsAgainstSignature(orderedArgs, sig)
	return sig.ReturnType, true
}

func (c *Checker) lookupClassInfo(name string) (classInfo, bool) {
	if info, ok := c.classes[name]; ok {
		return info, true
	}
	if info, ok := c.importedClasses[name]; ok {
		return info, true
	}
	return classInfo{}, false
}

func (c *Checker) iterableElementType(t *Type) *Type {
	if isUnknown(t) {
		return unknownType
	}
	if t.Name == "Option" && len(t.Args) == 1 {
		return t.Args[0]
	}
	if t.Name == "Array" && len(t.Args) == 1 {
		return t.Args[0]
	}
	if t.Kind == TypeInterface {
		if t.Name == "Iterable" && len(t.Args) == 1 {
			return t.Args[0]
		}
		if info, ok := c.interfaces[t.Name]; ok {
			subst := c.substForDecl(info.decl.TypeParameters, t.Args)
			if elem := c.iterableTypeFromRefs(info.decl.Extends, subst); !isUnknown(elem) {
				return elem
			}
		}
	}
	if t.Kind == TypeClass {
		if info, ok := c.classes[t.Name]; ok {
			subst := c.substForDecl(info.decl.TypeParameters, t.Args)
			if elem := c.iterableTypeFromRefs(info.decl.Implements, subst); !isUnknown(elem) {
				return elem
			}
		}
	}
	if t.Kind == TypeObject {
		if info, ok := c.lookupObjectInfo(t.Name); ok {
			subst := c.substForDecl(info.decl.TypeParameters, t.Args)
			if elem := c.iterableTypeFromRefs(info.decl.Implements, subst); !isUnknown(elem) {
				return elem
			}
		}
	}
	return unknownType
}

func (c *Checker) optionElementType(t *Type) *Type {
	if isUnknown(t) {
		return unknownType
	}
	if t.Name == "Option" && len(t.Args) == 1 {
		return t.Args[0]
	}
	return unknownType
}

func (c *Checker) optionType(elem *Type) *Type {
	optionType := &Type{Kind: TypeInterface, Name: "Option", Args: []*Type{elem}}
	if _, ok := c.lookupClassInfo("Option"); ok {
		optionType = &Type{Kind: TypeClass, Name: "Option", Args: []*Type{elem}}
	}
	return optionType
}

func (c *Checker) unwrappableSuccessType(t *Type) (*Type, bool) {
	if isUnknown(t) {
		return unknownType, true
	}
	switch t.Name {
	case "Option":
		if len(t.Args) == 1 {
			return t.Args[0], true
		}
	case "Result":
		if len(t.Args) == 2 {
			return t.Args[0], true
		}
	case "Either":
		if len(t.Args) == 2 {
			return t.Args[1], true
		}
	}
	return unknownType, false
}

func (c *Checker) shortCircuitCompatible(source, target *Type) bool {
	if isUnknown(source) || isUnknown(target) {
		return true
	}
	if source.Name != target.Name {
		return false
	}
	switch source.Name {
	case "Option":
		return len(source.Args) == 1 && len(target.Args) == 1
	case "Result":
		return len(source.Args) == 2 && len(target.Args) == 2 && sameType(source.Args[1], target.Args[1])
	case "Either":
		return len(source.Args) == 2 && len(target.Args) == 2 && sameType(source.Args[0], target.Args[0])
	default:
		return false
	}
}

func (c *Checker) interfaceArgsForType(t *Type, target string) ([]*Type, bool) {
	switch t.Kind {
	case TypeClass:
		info, ok := c.classes[t.Name]
		if !ok {
			return nil, false
		}
		subst := c.substForDecl(info.decl.TypeParameters, t.Args)
		return c.interfaceArgsFromRefs(info.decl.Implements, subst, target)
	case TypeInterface:
		if t.Name == target {
			return t.Args, true
		}
		info, ok := c.interfaces[t.Name]
		if !ok {
			return nil, false
		}
		subst := c.substForDecl(info.decl.TypeParameters, t.Args)
		return c.interfaceArgsFromRefs(info.decl.Extends, subst, target)
	default:
		return nil, false
	}
}

func (c *Checker) interfaceArgsFromRefs(refs []*parser.TypeRef, subst map[string]*Type, target string) ([]*Type, bool) {
	for _, ref := range refs {
		inst := c.instantiateTypeRef(ref, subst)
		if inst.Name == target {
			return inst.Args, true
		}
		if inst.Kind == TypeInterface {
			if args, ok := c.interfaceArgsForType(inst, target); ok {
				return args, true
			}
		}
	}
	return nil, false
}

func (c *Checker) iterableTypeFromRefs(refs []*parser.TypeRef, subst map[string]*Type) *Type {
	for _, ref := range refs {
		inst := c.instantiateTypeRef(ref, subst)
		if inst.Name == "Iterable" && len(inst.Args) == 1 {
			return inst.Args[0]
		}
		if inst.Kind == TypeInterface {
			if info, ok := c.interfaces[inst.Name]; ok {
				nextSubst := c.substForDecl(info.decl.TypeParameters, inst.Args)
				if elem := c.iterableTypeFromRefs(info.decl.Extends, nextSubst); !isUnknown(elem) {
					return elem
				}
			}
		}
	}
	return unknownType
}

func (c *Checker) supportsEquality(t *Type) bool {
	if isUnknown(t) {
		return true
	}
	switch t.Kind {
	case TypeBuiltin:
		switch t.Name {
		case "Int", "Int64", "Bool", "Str", "Rune", "Float", "Float64":
			return true
		default:
			return false
		}
	case TypeClass:
		if info, ok := c.classes[t.Name]; ok && info.decl.Enum {
			return true
		}
		return c.classImplementsEq(t)
	default:
		return false
	}
}

func (c *Checker) classImplementsEq(t *Type) bool {
	info, ok := c.classes[t.Name]
	if !ok {
		return false
	}
	for _, impl := range info.decl.Implements {
		if impl.Name != "Eq" || len(impl.Arguments) != 1 {
			continue
		}
		expected := c.instantiateTypeRef(impl.Arguments[0], c.substForDecl(info.decl.TypeParameters, t.Args))
		if sameType(expected, t) {
			return true
		}
	}
	return false
}

func (c *Checker) requireAssignable(actual, expected *Type, span parser.Span, code, message string) {
	if isUnknown(actual) || isUnknown(expected) {
		return
	}
	if !c.isAssignable(actual, expected) {
		c.addDiagnostic(code, message, span)
	}
}

func (c *Checker) isAssignable(actual, expected *Type) bool {
	if sameType(actual, expected) {
		return true
	}
	if expected.Kind == TypeRecord {
		if actual.Kind != TypeRecord {
			return false
		}
		for _, expectedField := range expected.Fields {
			found := false
			for _, actualField := range actual.Fields {
				if actualField.Name != expectedField.Name {
					continue
				}
				if !c.isAssignable(actualField.Type, expectedField.Type) {
					return false
				}
				found = true
				break
			}
			if !found {
				return false
			}
		}
		return true
	}
	if expected.Kind == TypeClass && actual.Kind == TypeRecord {
		return false
	}
	if expected.Kind != TypeInterface {
		return false
	}
	switch actual.Kind {
	case TypeClass:
		info, ok := c.classes[actual.Name]
		if !ok {
			return false
		}
		subst := c.substForDecl(info.decl.TypeParameters, actual.Args)
		for _, impl := range info.decl.Implements {
			inst := c.instantiateTypeRef(impl, subst)
			if sameType(inst, expected) || c.interfaceAssignable(inst, expected, map[string]bool{}) {
				return true
			}
		}
	case TypeObject:
		info, ok := c.lookupObjectInfo(actual.Name)
		if !ok {
			return false
		}
		subst := c.substForDecl(info.decl.TypeParameters, actual.Args)
		for _, impl := range info.decl.Implements {
			inst := c.instantiateTypeRef(impl, subst)
			if sameType(inst, expected) || c.interfaceAssignable(inst, expected, map[string]bool{}) {
				return true
			}
		}
	case TypeInterface:
		return c.interfaceAssignable(actual, expected, map[string]bool{})
	}
	return false
}

func (c *Checker) recordAssignableToFields(actualFields, expectedFields []RecordField) bool {
	if len(actualFields) != len(expectedFields) {
		return false
	}
	actualByName := make(map[string]*Type, len(actualFields))
	for i := range actualFields {
		actualByName[actualFields[i].Name] = actualFields[i].Type
	}
	for _, expectedField := range expectedFields {
		actualType, ok := actualByName[expectedField.Name]
		if !ok {
			return false
		}
		if !c.isAssignable(actualType, expectedField.Type) {
			return false
		}
	}
	return true
}

func implicitVisibleRecordFields(class *parser.ClassDecl) []parser.FieldDecl {
	fields := make([]parser.FieldDecl, 0, len(class.Fields))
	for _, field := range class.Fields {
		if constructorVisibleField(field) {
			fields = append(fields, field)
		}
	}
	return fields
}

func classAnonymousRecordShape(class *parser.ClassDecl, subst map[string]*Type, c *Checker) (required []RecordField, optional []RecordField, ok bool) {
	for _, field := range class.Fields {
		typed := RecordField{Name: field.Name, Type: c.instantiateTypeRef(field.Type, subst)}
		if field.Private {
			if field.Initializer == nil {
				return nil, nil, false
			}
			continue
		}
		if field.Initializer != nil {
			optional = append(optional, typed)
			continue
		}
		required = append(required, typed)
	}
	return required, optional, true
}

func recordMatchesVisibleShape(actualFields, requiredFields, optionalFields []RecordField) bool {
	if len(actualFields) < len(requiredFields) || len(actualFields) > len(requiredFields)+len(optionalFields) {
		return false
	}
	optionalByName := make(map[string]*Type, len(optionalFields))
	for i := range optionalFields {
		optionalByName[optionalFields[i].Name] = optionalFields[i].Type
	}
	actualByName := make(map[string]*Type, len(actualFields))
	for i := range actualFields {
		actualByName[actualFields[i].Name] = actualFields[i].Type
	}
	for _, required := range requiredFields {
		actual, ok := actualByName[required.Name]
		if !ok || !sameType(actual, required.Type) {
			return false
		}
	}
	for name, actual := range actualByName {
		foundRequired := false
		for _, required := range requiredFields {
			if required.Name == name {
				foundRequired = true
				break
			}
		}
		if foundRequired {
			continue
		}
		expected, ok := optionalByName[name]
		if !ok || !sameType(actual, expected) {
			return false
		}
	}
	return true
}

func (c *Checker) interfaceAssignable(actual, expected *Type, seen map[string]bool) bool {
	if sameType(actual, expected) {
		return true
	}
	if actual == nil || actual.Kind != TypeInterface {
		return false
	}
	key := actual.Name + actual.String()
	if seen[key] {
		return false
	}
	seen[key] = true
	info, ok := c.interfaces[actual.Name]
	if !ok {
		return false
	}
	subst := c.substForDecl(info.decl.TypeParameters, actual.Args)
	for _, parent := range info.decl.Extends {
		inst := c.instantiateTypeRef(parent, subst)
		if sameType(inst, expected) || c.interfaceAssignable(inst, expected, seen) {
			return true
		}
	}
	return false
}

func (c *Checker) typeParametersForName(name string) ([]parser.TypeParameter, bool) {
	if info, ok := c.classes[name]; ok {
		return info.decl.TypeParameters, true
	}
	if info, ok := c.importedClasses[name]; ok {
		return info.decl.TypeParameters, true
	}
	if info, ok := c.interfaces[name]; ok {
		return info.decl.TypeParameters, true
	}
	if info, ok := c.importedInterfaces[name]; ok {
		return info.decl.TypeParameters, true
	}
	return nil, false
}

func (c *Checker) typeArgsInOrder(params []parser.TypeParameter, subst map[string]*Type) []*Type {
	args := make([]*Type, len(params))
	for i, param := range params {
		if subst != nil {
			args[i] = subst[param.Name]
		}
		if args[i] == nil {
			args[i] = unknownType
		}
	}
	return args
}

func (c *Checker) checkTypeArgBounds(args []*Type, params []parser.TypeParameter, span parser.Span) bool {
	if len(args) != len(params) {
		return false
	}
	subst := c.substForDecl(params, args)
	ok := true
	for i, param := range params {
		for _, bound := range param.Bounds {
			expected := c.instantiateTypeRef(bound, subst)
			if !c.typeSatisfiesBound(args[i], expected) {
				c.addDiagnostic("type_argument_bound", "type argument "+args[i].String()+" does not satisfy bound "+expected.String()+" for '"+param.Name+"'", span)
				ok = false
			}
		}
	}
	return ok
}

func (c *Checker) typeSatisfiesBound(actual, bound *Type) bool {
	if isUnknown(actual) || isUnknown(bound) {
		return true
	}
	if c.isAssignable(actual, bound) {
		return true
	}
	if bound.Kind == TypeInterface && c.hasBoundWitness(bound) {
		return true
	}
	return false
}

func (c *Checker) hasBoundWitness(expected *Type) bool {
	for _, info := range c.classes {
		if c.isAssignable(&Type{Kind: TypeClass, Name: info.name, Args: nil}, expected) {
			return true
		}
	}
	for _, info := range c.objects {
		if c.isAssignable(&Type{Kind: TypeObject, Name: info.name, Args: nil}, expected) {
			return true
		}
	}
	return false
}

func (c *Checker) addDiagnostic(code, message string, span parser.Span) {
	c.diagnostics = append(c.diagnostics, semantic.Diagnostic{Code: code, Message: message, Span: span})
}

func (c *Checker) define(name string, typ *Type, mutable bool) {
	c.currentScope()[name] = binding{typ: typ, mutable: mutable}
}

func (c *Checker) lookup(name string) (binding, bool) {
	b, _, ok := c.lookupWithDepth(name)
	return b, ok
}

func (c *Checker) lookupWithDepth(name string) (binding, int, bool) {
	for i := len(c.scopes) - 1; i >= 0; i-- {
		if value, ok := c.scopes[i][name]; ok {
			return value, i, true
		}
	}
	if value, ok := c.globals[name]; ok {
		return value, -1, true
	}
	return binding{}, -1, false
}

func (c *Checker) capturesMutableOuterBinding(b binding, depth int) bool {
	if len(c.lambdaScopes) == 0 || !b.mutable {
		return false
	}
	boundary := c.lambdaScopes[len(c.lambdaScopes)-1]
	if depth == -1 {
		return true
	}
	return depth < boundary
}

func (c *Checker) pushScope() { c.scopes = append(c.scopes, scope{}) }
func (c *Checker) popScope()  { c.scopes = c.scopes[:len(c.scopes)-1] }

func (c *Checker) currentScope() scope {
	if len(c.scopes) == 0 {
		c.pushScope()
	}
	return c.scopes[len(c.scopes)-1]
}

func (c *Checker) pushTypeScope() { c.typeScopes = append(c.typeScopes, typeScope{}) }
func (c *Checker) popTypeScope()  { c.typeScopes = c.typeScopes[:len(c.typeScopes)-1] }

func (c *Checker) currentTypeScope() typeScope {
	if len(c.typeScopes) == 0 {
		c.pushTypeScope()
	}
	return c.typeScopes[len(c.typeScopes)-1]
}

func (c *Checker) kindOf(name string) TypeKind {
	for i := len(c.typeScopes) - 1; i >= 0; i-- {
		if kind, ok := c.typeScopes[i][name]; ok {
			return kind
		}
	}
	if _, ok := c.classes[name]; ok {
		return TypeClass
	}
	if _, ok := c.importedClasses[name]; ok {
		return TypeClass
	}
	if _, ok := c.interfaces[name]; ok {
		return TypeInterface
	}
	if _, ok := c.importedInterfaces[name]; ok {
		return TypeInterface
	}
	if isBuiltinInterfaceType(name) {
		return TypeInterface
	}
	if isBuiltinType(name) {
		return TypeBuiltin
	}
	return ""
}

func functionType(name string, sig Signature) *Type {
	return &Type{Kind: TypeFunction, Name: name, Signature: &sig}
}

func builtin(name string) *Type { return &Type{Kind: TypeBuiltin, Name: name} }

func isUnitType(t *Type) bool {
	return t != nil && t.Kind == TypeBuiltin && t.Name == "Unit"
}

func isBuiltinType(name string) bool {
	switch name {
	case "Int", "Int64", "Bool", "Str", "Rune", "Float", "Float64", "Array", "Unit":
		return true
	default:
		return false
	}
}

func isBuiltinInterfaceType(name string) bool {
	switch name {
	case "Eq", "Ordering", "Iterable", "Iterator", "List", "Set", "Map", "Printer", "Option", "Result", "Either":
		return true
	default:
		return false
	}
}

func isBuiltinValue(name string) bool {
	switch name {
	case "List", "Map", "Set", "Array", "Range", "Some", "None", "Ok", "Err", "Left", "Right":
		return true
	default:
		return false
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

func (c *Checker) implicitOSMethodSignature(name string) (Signature, bool) {
	registry, err := predef.Load()
	if err != nil {
		panic(err)
	}
	seen := map[string]bool{}
	var lookup func(typeName, methodName string) (predef.MethodDescriptor, bool)
	lookup = func(typeName, methodName string) (predef.MethodDescriptor, bool) {
		if seen[typeName] {
			return predef.MethodDescriptor{}, false
		}
		seen[typeName] = true
		descriptor, ok := registry.Types[typeName]
		if !ok {
			return predef.MethodDescriptor{}, false
		}
		for _, method := range descriptor.Methods {
			if method.Name == methodName && !method.Private && !method.Constructor {
				return method, true
			}
		}
		for _, iface := range descriptor.ImplementedInterfaces {
			if iface == nil || iface.Name == "" {
				continue
			}
			if method, ok := lookup(iface.Name, methodName); ok {
				return method, true
			}
		}
		return predef.MethodDescriptor{}, false
	}
	method, ok := lookup("OS", name)
	if !ok {
		return Signature{}, false
	}
	params := make([]*Type, len(method.Parameters))
	for i, param := range method.Parameters {
		params[i] = fromTypeRef(param.Type, c)
	}
	return Signature{
		Parameters: params,
		ReturnType: fromTypeRef(method.ReturnType, c),
		Variadic:   len(method.Parameters) > 0 && method.Parameters[len(method.Parameters)-1].Variadic,
	}, true
}

func isNumeric(t *Type) bool {
	if isUnknown(t) {
		return true
	}
	switch t.Name {
	case "Int", "Int64", "Float", "Float64":
		return true
	default:
		return false
	}
}

func isOrdered(t *Type) bool {
	if isUnknown(t) {
		return true
	}
	switch t.Name {
	case "Int", "Int64", "Float", "Float64", "Str", "Rune":
		return true
	default:
		return false
	}
}

func exprSpan(expr parser.Expr) parser.Span {
	switch e := expr.(type) {
	case *parser.Identifier:
		return e.Span
	case *parser.PlaceholderExpr:
		return e.Span
	case *parser.IntegerLiteral:
		return e.Span
	case *parser.FloatLiteral:
		return e.Span
	case *parser.RuneLiteral:
		return e.Span
	case *parser.BoolLiteral:
		return e.Span
	case *parser.StringLiteral:
		return e.Span
	case *parser.UnitLiteral:
		return e.Span
	case *parser.ListLiteral:
		return e.Span
	case *parser.CallExpr:
		return e.Span
	case *parser.MemberExpr:
		return e.Span
	case *parser.IndexExpr:
		return e.Span
	case *parser.RecordUpdateExpr:
		return e.Span
	case *parser.IfExpr:
		return e.Span
	case *parser.ForYieldExpr:
		return e.Span
	case *parser.LambdaExpr:
		return e.Span
	case *parser.BinaryExpr:
		return e.Span
	case *parser.IsExpr:
		return e.Span
	case *parser.UnaryExpr:
		return e.Span
	case *parser.GroupExpr:
		return e.Span
	case *parser.BlockExpr:
		return e.Span
	default:
		return parser.Span{}
	}
}

func blockSpan(block *parser.BlockStmt) parser.Span {
	if block == nil {
		return parser.Span{}
	}
	return block.Span
}

func stmtSpan(stmt parser.Statement) parser.Span {
	switch s := stmt.(type) {
	case *parser.ValStmt:
		return s.Span
	case *parser.LocalFunctionStmt:
		return s.Span
	case *parser.AssignmentStmt:
		return s.Span
	case *parser.MultiAssignmentStmt:
		return s.Span
	case *parser.IfStmt:
		return s.Span
	case *parser.WhileStmt:
		return s.Span
	case *parser.ForStmt:
		return s.Span
	case *parser.ReturnStmt:
		return s.Span
	case *parser.BreakStmt:
		return s.Span
	case *parser.ExprStmt:
		return s.Span
	default:
		return parser.Span{}
	}
}
