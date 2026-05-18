package interpreter

import (
	"fmt"
	"math"
	"strconv"
	"strings"

	"a-lang/module"
	"a-lang/parser"
	"a-lang/predef"
)

type Value any

type deferredValue struct{}

type RuntimeError struct {
	Message string
	Span    parser.Span
}

func (e RuntimeError) Error() string {
	return fmt.Sprintf("%s at %d:%d", e.Message, e.Span.Start.Line, e.Span.Start.Column)
}

type slot struct {
	value   Value
	mutable bool
}

type env struct {
	parent *env
	values map[string]slot
}

type Interpreter struct {
	program       *parser.Program
	functions     map[string]*parser.FunctionDecl
	classes       map[string]*parser.ClassDecl
	objects       map[string]*parser.ClassDecl
	interfaces    map[string]*parser.InterfaceDecl
	enumCases     map[string]string
	imports       map[string]*Interpreter
	directImports map[string]runtimeImportedSymbol
	publicGlobals map[string]bool
	globals       *env
	ready         bool
}

type runtimeImportedSymbol struct {
	module      *Interpreter
	original    string
	objectName  string
	isInterface bool
	isFunction  bool
	isValue     bool
}

type instance struct {
	class    *parser.ClassDecl
	caseName string
	fields   map[string]Value
}

type closure struct {
	params             []string
	variadic           bool
	tupleDestructuring bool
	body               parser.Expr
	blockBody          *parser.BlockStmt
	returnType         *parser.TypeRef
	env                *env
}

type builtinRef struct{ name string }

type nativeList struct{ items []Value }
type nativeListIterator struct {
	items []Value
	index int
}
type nativeArray struct{ items []Value }
type nativeTuple struct {
	items []Value
	names []string
}
type nativeRecord struct {
	fields map[string]Value
	order  []string
}
type nativeOption struct {
	value Value
	set   bool
}
type nativeResult struct {
	value Value
	err   Value
	ok    bool
}
type nativeEither struct {
	left     Value
	right    Value
	rightSet bool
}
type nativeSet struct {
	keys  map[string]Value
	order []string
}
type nativeMap struct {
	items map[string]Value
	keys  map[string]Value
	order []string
}
type nativePrinter struct {
	stderr bool
}
type nativeOS struct {
	stdout *nativePrinter
	stderr *nativePrinter
}

type returnSignal struct {
	value Value
}

type breakSignal struct{}

func (t *nativeTuple) String() string {
	parts := make([]string, len(t.items))
	for i, item := range t.items {
		if i < len(t.names) && t.names[i] != "" {
			parts[i] = t.names[i] + "=" + fmt.Sprint(item)
		} else {
			parts[i] = fmt.Sprint(item)
		}
	}
	return "(" + strings.Join(parts, ", ") + ")"
}

func (r *nativeRecord) String() string {
	parts := make([]string, 0, len(r.order))
	for _, name := range r.order {
		parts = append(parts, name+"="+fmt.Sprint(r.fields[name]))
	}
	return "{" + strings.Join(parts, ", ") + "}"
}

func New(program *parser.Program) *Interpreter {
	in := &Interpreter{
		program:       program,
		functions:     map[string]*parser.FunctionDecl{},
		classes:       map[string]*parser.ClassDecl{},
		objects:       map[string]*parser.ClassDecl{},
		interfaces:    map[string]*parser.InterfaceDecl{},
		enumCases:     map[string]string{},
		imports:       map[string]*Interpreter{},
		directImports: map[string]runtimeImportedSymbol{},
		publicGlobals: map[string]bool{},
	}
	in.installAmbientPredef()
	for _, fn := range program.Functions {
		in.functions[fn.Name] = fn
	}
	for _, iface := range program.Interfaces {
		in.interfaces[iface.Name] = iface
	}
	for _, class := range program.Classes {
		if class.Object {
			in.objects[class.Name] = class
		} else {
			in.classes[class.Name] = class
		}
		if class.Enum {
			for _, enumCase := range class.Cases {
				in.enumCases[enumCase.Name] = class.Name
			}
		}
	}
	for _, stmt := range program.Statements {
		valStmt, ok := stmt.(*parser.ValStmt)
		if !ok || !valStmt.Public {
			continue
		}
		for _, binding := range valStmt.Bindings {
			if binding.Name != "_" {
				in.publicGlobals[binding.Name] = true
			}
		}
	}
	return in
}

func (in *Interpreter) installAmbientPredef() {
	registry := builtinRegistry()
	for name, decl := range registry.Interfaces {
		descriptor, ok := registry.Types[name]
		if !ok || !descriptor.Directives.Interpreter {
			continue
		}
		in.interfaces[name] = decl
	}
	for name, decl := range registry.Classes {
		descriptor, ok := registry.Types[name]
		if !ok || !descriptor.Directives.Interpreter {
			continue
		}
		if decl.Object {
			in.objects[name] = decl
		} else {
			in.classes[name] = decl
			if decl.Enum {
				for _, enumCase := range decl.Cases {
					in.enumCases[enumCase.Name] = decl.Name
				}
			}
		}
	}
}

func NewModule(mod *module.LoadedModule) *Interpreter {
	in := New(mod.Program)
	for alias, imported := range mod.Imports {
		in.imports[alias] = NewModule(imported)
	}
	for localName, symbol := range mod.SymbolImports {
		in.directImports[localName] = runtimeImportedSymbol{
			module:      NewModule(symbol.Module),
			original:    symbol.OriginalName,
			objectName:  symbol.ObjectName,
			isInterface: symbol.IsInterface,
			isFunction:  symbol.IsFunction,
			isValue:     symbol.IsValue,
		}
	}
	return in
}

func (in *Interpreter) Call(function string, args ...Value) (Value, error) {
	global, err := in.ensureGlobals()
	if err != nil {
		return nil, err
	}
	return in.callFunctionByName(function, args, global)
}

func (in *Interpreter) ensureGlobals() (*env, error) {
	if in.ready {
		return in.globals, nil
	}
	global := newEnv(nil)
	if err := in.execTopLevel(global); err != nil {
		return nil, err
	}
	in.globals = global
	in.ready = true
	return global, nil
}

func (in *Interpreter) execTopLevel(global *env) error {
	for alias, imported := range in.imports {
		global.define(alias, moduleRef{name: alias}, false)
		if _, err := imported.ensureGlobals(); err != nil {
			return err
		}
	}
	for localName, symbol := range in.directImports {
		if _, err := symbol.module.ensureGlobals(); err != nil {
			return err
		}
		if symbol.isInterface {
			continue
		}
		if symbol.isFunction {
			if symbol.objectName != "" {
				value, ok := symbol.module.globals.get(symbol.objectName)
				if !ok {
					return RuntimeError{Message: "undefined object '" + symbol.objectName + "'", Span: parser.Span{}}
				}
				obj, ok := value.value.(*instance)
				if !ok {
					return RuntimeError{Message: "object '" + symbol.objectName + "' is not an instance", Span: parser.Span{}}
				}
				global.define(localName, boundMethodRef{receiver: obj, name: symbol.original}, false)
				continue
			}
			global.define(localName, functionRef{module: symbol.module, name: symbol.original}, false)
			continue
		}
		if symbol.isValue {
			if value, ok := symbol.module.globals.get(symbol.original); ok {
				global.define(localName, value.value, false)
				continue
			}
		}
		if class, ok := symbol.module.objects[symbol.original]; ok {
			if value, ok := symbol.module.globals.get(symbol.original); ok {
				global.define(localName, value.value, false)
				continue
			}
			_ = class
		}
		global.define(localName, classRef{module: symbol.module, name: symbol.original}, false)
	}
	for name, class := range in.objects {
		if _, exists := global.get(name); exists {
			continue
		}
		if name == "OS" {
			global.define(name, newNativeOS(), false)
			continue
		}
		value, err := in.construct(class, nil, global)
		if err != nil {
			return err
		}
		global.define(name, value, false)
	}
	for _, stmt := range in.program.Statements {
		if _, signal, err := in.execStmt(stmt, global, nil); err != nil {
			return err
		} else if signal != nil {
			return RuntimeError{Message: "unexpected control flow at top level", Span: stmtSpan(stmt)}
		}
	}
	return nil
}

func (in *Interpreter) callFunctionByName(name string, args []Value, parent *env) (Value, error) {
	fn, ok := in.functions[name]
	if !ok {
		return nil, RuntimeError{Message: "undefined function '" + name + "'", Span: parser.Span{}}
	}
	return in.callFunction(fn, args, parent)
}

func (in *Interpreter) callFunction(fn *parser.FunctionDecl, args []Value, parent *env) (Value, error) {
	if !acceptsArgCount(fn.Parameters, len(args)) {
		return nil, RuntimeError{Message: fmt.Sprintf("function '%s' expects %s args, got %d", fn.Name, expectedCallableArgs(fn.Parameters), len(args)), Span: fn.Span}
	}
	local := newEnv(parent)
	for i, param := range fn.Parameters {
		if param.Variadic {
			local.define(param.Name, &nativeList{items: append([]Value{}, args[i:]...)}, false)
			break
		}
		local.define(param.Name, args[i], false)
	}
	value, signal, err := in.execBlock(fn.Body, local, nil)
	if err != nil {
		return nil, err
	}
	if ret, ok := signal.(returnSignal); ok {
		return in.coerceValueForTypeRef(fn.ReturnType, ret.value), nil
	}
	if fn.ReturnType == nil || isUnitTypeRef(fn.ReturnType) {
		return nil, nil
	}
	return in.coerceValueForTypeRef(fn.ReturnType, value), nil
}

func (in *Interpreter) callMethod(receiver *instance, method *parser.MethodDecl, args []Value, parent *env) (Value, error) {
	if !acceptsArgCount(method.Parameters, len(args)) {
		return nil, RuntimeError{Message: fmt.Sprintf("method '%s' expects %s args, got %d", method.Name, expectedCallableArgs(method.Parameters), len(args)), Span: method.Span}
	}
	local := newEnv(parent)
	local.define("this", receiver, false)
	if receiver.class.Enum {
		for _, enumCase := range receiver.class.Cases {
			if len(enumCase.Fields) == 0 {
				value, err := in.constructEnumCase(receiver.class, enumCase, nil, parent, enumCase.Span)
				if err != nil {
					return nil, err
				}
				local.define(enumCase.Name, value, false)
			}
		}
	}
	if method.Constructor {
		local.define("__constructor__", true, false)
	}
	for i, param := range method.Parameters {
		if param.Variadic {
			local.define(param.Name, &nativeList{items: append([]Value{}, args[i:]...)}, false)
			break
		}
		local.define(param.Name, args[i], false)
	}
	value, signal, err := in.execBlock(method.Body, local, receiver)
	if err != nil {
		return nil, err
	}
	if ret, ok := signal.(returnSignal); ok {
		return in.coerceValueForTypeRef(method.ReturnType, ret.value), nil
	}
	if method.ReturnType == nil || isUnitTypeRef(method.ReturnType) {
		return nil, nil
	}
	return in.coerceValueForTypeRef(method.ReturnType, value), nil
}

func (in *Interpreter) callInterfaceMethod(receiver *instance, method parser.InterfaceMethod, args []Value, parent *env) (Value, error) {
	if !acceptsArgCount(method.Parameters, len(args)) {
		return nil, RuntimeError{Message: fmt.Sprintf("method '%s' expects %s args, got %d", method.Name, expectedCallableArgs(method.Parameters), len(args)), Span: method.Span}
	}
	if method.Body == nil {
		return nil, RuntimeError{Message: "interface method '" + method.Name + "' has no default body", Span: method.Span}
	}
	local := newEnv(parent)
	local.define("this", receiver, false)
	for i, param := range method.Parameters {
		if param.Variadic {
			local.define(param.Name, &nativeList{items: append([]Value{}, args[i:]...)}, false)
			break
		}
		local.define(param.Name, args[i], false)
	}
	value, signal, err := in.execBlock(method.Body, local, receiver)
	if err != nil {
		return nil, err
	}
	if ret, ok := signal.(returnSignal); ok {
		return in.coerceValueForTypeRef(method.ReturnType, ret.value), nil
	}
	if method.ReturnType == nil || isUnitTypeRef(method.ReturnType) {
		return nil, nil
	}
	return in.coerceValueForTypeRef(method.ReturnType, value), nil
}

func enumCaseMethods(class *parser.ClassDecl, caseName string) []*parser.MethodDecl {
	if !class.Enum || caseName == "" {
		return nil
	}
	for i := range class.Cases {
		if class.Cases[i].Name == caseName {
			return class.Cases[i].Methods
		}
	}
	return nil
}

func instanceMethods(obj *instance) []*parser.MethodDecl {
	methods := append([]*parser.MethodDecl{}, enumCaseMethods(obj.class, obj.caseName)...)
	methods = append(methods, obj.class.Methods...)
	return methods
}

func implicitConstructorFields(class *parser.ClassDecl) []parser.FieldDecl {
	if !hasSafeImplicitRuntimeConstructor(class) {
		return nil
	}
	fields := make([]parser.FieldDecl, 0, len(class.Fields))
	for _, field := range class.Fields {
		if !field.Private && field.Initializer == nil {
			fields = append(fields, field)
		}
	}
	return fields
}

func hasSafeImplicitRuntimeConstructor(class *parser.ClassDecl) bool {
	for _, field := range class.Fields {
		if field.Mutable {
			continue
		}
		if !field.Private && field.Initializer == nil {
			continue
		}
		if field.Initializer != nil {
			continue
		}
		return false
	}
	return true
}

func (in *Interpreter) applyImplicitConstructor(receiver *instance, args []Value, span parser.Span) error {
	fields := implicitConstructorFields(receiver.class)
	if len(args) != len(fields) {
		return RuntimeError{Message: fmt.Sprintf("constructor '%s' expects %d args, got %d", receiver.class.Name, len(fields), len(args)), Span: span}
	}
	for i, field := range fields {
		receiver.fields[field.Name] = args[i]
	}
	return nil
}

func constructorParamOptions(class *parser.ClassDecl) [][]parser.Parameter {
	var options [][]parser.Parameter
	for _, method := range class.Methods {
		if method.Constructor {
			options = append(options, method.Parameters)
		}
	}
	fields := implicitConstructorFields(class)
	params := make([]parser.Parameter, len(fields))
	for i, field := range fields {
		params[i] = parser.Parameter{Name: field.Name, Type: field.Type, Span: field.Span}
	}
	options = append(options, params)
	return options
}

func reorderConstructorValueArgs(class *parser.ClassDecl, args []namedValueArg, span parser.Span) ([]Value, error) {
	var firstErr error
	for _, params := range constructorParamOptions(class) {
		reordered, err := reorderNamedValueArgs(params, args, span, "constructor '"+class.Name+"'")
		if err == nil {
			return reordered, nil
		}
		if firstErr == nil {
			firstErr = err
		}
	}
	if firstErr != nil {
		return nil, firstErr
	}
	return namedArgValues(args), nil
}

func (in *Interpreter) constructInto(receiver *instance, class *parser.ClassDecl, args []Value, parent *env, span parser.Span) error {
	for _, method := range class.Methods {
		if method.Constructor && in.runtimeMethodMatches(method, args) {
			_, err := in.callMethod(receiver, method, args, parent)
			return err
		}
	}
	if len(args) == 1 {
		if record, ok := args[0].(*nativeRecord); ok {
			if in.recordMatchesVisibleClassShape(class, record) {
				for _, field := range class.Fields {
					if value, ok := record.fields[field.Name]; ok {
						receiver.fields[field.Name] = value
					}
				}
				return nil
			}
		}
	}
	if len(implicitConstructorFields(class)) > 0 || len(args) == 0 {
		return in.applyImplicitConstructor(receiver, args, span)
	}
	return RuntimeError{Message: fmt.Sprintf("constructor '%s' expects 0 args, got %d", class.Name, len(args)), Span: span}
}

func (in *Interpreter) construct(class *parser.ClassDecl, args []Value, parent *env) (Value, error) {
	obj := &instance{class: class, fields: map[string]Value{}}
	fieldEnv := newEnv(parent)
	fieldEnv.define("this", obj, false)
	for _, field := range class.Fields {
		switch {
		case field.Initializer != nil:
			value, err := in.evalExpr(field.Initializer, fieldEnv)
			if err != nil {
				return nil, err
			}
			obj.fields[field.Name] = value
		case field.Deferred:
			obj.fields[field.Name] = deferredValue{}
		default:
			obj.fields[field.Name] = zeroValue(field.Type)
		}
	}
	if err := in.constructInto(obj, class, args, parent, class.Span); err != nil {
		return nil, err
	}
	return obj, nil
}

func (in *Interpreter) constructEnumCase(class *parser.ClassDecl, enumCase parser.EnumCaseDecl, args []Value, parent *env, span parser.Span) (Value, error) {
	obj := &instance{class: class, caseName: enumCase.Name, fields: map[string]Value{}}
	fieldEnv := newEnv(parent)
	fieldEnv.define("this", obj, false)
	for _, field := range class.Fields {
		switch {
		case field.Initializer != nil:
			value, err := in.evalExpr(field.Initializer, fieldEnv)
			if err != nil {
				return nil, err
			}
			obj.fields[field.Name] = value
		case field.Deferred:
			obj.fields[field.Name] = deferredValue{}
		default:
			obj.fields[field.Name] = zeroValue(field.Type)
		}
	}
	for _, assignment := range enumCase.Assignments {
		value, err := in.evalExpr(assignment.Value, fieldEnv)
		if err != nil {
			return nil, err
		}
		obj.fields[assignment.Name] = value
	}
	if len(args) != len(enumCase.Fields) {
		return nil, RuntimeError{Message: fmt.Sprintf("enum case '%s' expects %d args, got %d", enumCase.Name, len(enumCase.Fields), len(args)), Span: span}
	}
	for i, field := range enumCase.Fields {
		obj.fields[field.Name] = args[i]
	}
	return obj, nil
}

func (in *Interpreter) execBlock(block *parser.BlockStmt, parent *env, self *instance) (Value, any, error) {
	local := newEnv(parent)
	var lastValue Value
	for _, stmt := range block.Statements {
		value, signal, err := in.execStmt(stmt, local, self)
		if err != nil || signal != nil {
			return value, signal, err
		}
		lastValue = value
	}
	return lastValue, nil, nil
}

func (in *Interpreter) execStmt(stmt parser.Statement, local *env, self *instance) (Value, any, error) {
	switch s := stmt.(type) {
	case *parser.ValStmt:
		values, err := in.bindingValues(s.Bindings, s.Values, local, s.Span)
		if err != nil {
			return nil, nil, err
		}
		for i, binding := range s.Bindings {
			if binding.Name == "_" {
				continue
			}
			var value Value
			if i < len(values) {
				value = values[i]
			}
			local.define(binding.Name, value, binding.Mutable)
		}
		return nil, nil, nil
	case *parser.LocalFunctionStmt:
		params := make([]string, len(s.Function.Parameters))
		for i, param := range s.Function.Parameters {
			params[i] = param.Name
		}
		local.define(s.Function.Name, &closure{params: params, variadic: len(s.Function.Parameters) > 0 && s.Function.Parameters[len(s.Function.Parameters)-1].Variadic, blockBody: s.Function.Body, returnType: s.Function.ReturnType, env: local}, false)
		return nil, nil, nil
	case *parser.AssignmentStmt:
		value, err := in.evalExpr(s.Value, local)
		if err != nil {
			return nil, nil, err
		}
		return nil, nil, in.assign(s.Target, s.Operator, value, local)
	case *parser.MultiAssignmentStmt:
		values, err := in.assignmentValues(len(s.Targets), s.Values, local, s.Span)
		if err != nil {
			return nil, nil, err
		}
		for i, target := range s.Targets {
			if err := in.assign(target, s.Operator, values[i], local); err != nil {
				return nil, nil, err
			}
		}
		return nil, nil, nil
	case *parser.UnwrapStmt:
		ok, sourceValue, err := in.execUnwrapBinding(s.Bindings, s.Value, s.Span, local)
		if err != nil {
			return nil, nil, err
		}
		if !ok {
			return nil, returnSignal{value: sourceValue}, nil
		}
		return nil, nil, nil
	case *parser.UnwrapBlockStmt:
		for _, clause := range s.Clauses {
			ok, sourceValue, err := in.execUnwrapBinding(clause.Bindings, clause.Value, clause.Span, local)
			if err != nil {
				return nil, nil, err
			}
			if !ok {
				return nil, returnSignal{value: sourceValue}, nil
			}
		}
		return nil, nil, nil
	case *parser.GuardStmt:
		ok, _, err := in.execUnwrapBinding(s.Bindings, s.Value, s.Span, local)
		if err != nil {
			return nil, nil, err
		}
		if !ok {
			value, signal, err := in.evalBlockValue(s.Fallback, local, self, "unwrap else block must end with a value-producing statement")
			if err != nil || signal != nil {
				return nil, signal, err
			}
			return nil, returnSignal{value: value}, nil
		}
		return nil, nil, nil
	case *parser.GuardBlockStmt:
		fallbackEnv := cloneEnvShallow(local)
		for _, clause := range s.Clauses {
			ok, _, err := in.execUnwrapBinding(clause.Bindings, clause.Value, clause.Span, local)
			if err != nil {
				return nil, nil, err
			}
			if !ok {
				value, signal, err := in.evalBlockValue(s.Fallback, fallbackEnv, self, "unwrap else block must end with a value-producing statement")
				if err != nil || signal != nil {
					return nil, signal, err
				}
				return nil, returnSignal{value: value}, nil
			}
		}
		return nil, nil, nil
	case *parser.IfStmt:
		if s.BindingValue != nil {
			optionValue, err := in.evalExpr(s.BindingValue, local)
			if err != nil {
				return nil, nil, err
			}
			set, value, err := in.optionBindingValue(optionValue, local, exprSpan(s.BindingValue))
			if err != nil {
				return nil, nil, err
			}
			if set {
				thenEnv := newEnv(local)
				boundValues, err := in.destructureBoundValue(s.Bindings, value, s.Span)
				if err != nil {
					return nil, nil, err
				}
				for i, binding := range s.Bindings {
					if binding.Name == "_" {
						continue
					}
					thenEnv.define(binding.Name, in.coerceValueForBinding(binding.Type, boundValues[i]), false)
				}
				return in.execBlock(s.Then, thenEnv, self)
			}
		} else {
			cond, err := in.evalExpr(s.Condition, local)
			if err != nil {
				return nil, nil, err
			}
			truthy, err := asBool(cond, exprSpan(s.Condition))
			if err != nil {
				return nil, nil, err
			}
			if truthy {
				return in.execBlock(s.Then, local, self)
			}
		}
		if s.ElseIf != nil {
			return in.execStmt(s.ElseIf, local, self)
		}
		if s.Else != nil {
			return in.execBlock(s.Else, local, self)
		}
		return nil, nil, nil
	case *parser.MatchStmt:
		value, err := in.evalExpr(s.Value, local)
		if err != nil {
			return nil, nil, err
		}
		for _, matchCase := range s.Cases {
			bindings, ok, err := in.matchPattern(matchCase.Pattern, value, local)
			if err != nil {
				return nil, nil, err
			}
			if !ok {
				continue
			}
			caseEnv := newEnv(local)
			for _, binding := range bindings {
				caseEnv.define(binding.name, binding.value, false)
			}
			if matchCase.Guard != nil {
				guardValue, err := in.evalExpr(matchCase.Guard, caseEnv)
				if err != nil {
					return nil, nil, err
				}
				truthy, err := asBool(guardValue, exprSpan(matchCase.Guard))
				if err != nil {
					return nil, nil, err
				}
				if !truthy {
					continue
				}
			}
			if matchCase.Body != nil {
				return in.execBlock(matchCase.Body, caseEnv, self)
			}
			if matchCase.Expr != nil {
				value, err := in.evalExpr(matchCase.Expr, caseEnv)
				return value, nil, err
			}
			return nil, nil, nil
		}
		if s.Partial {
			return nil, nil, nil
		}
		return nil, nil, nil
	case *parser.WhileStmt:
		return in.execWhile(s, local, self)
	case *parser.ForStmt:
		return in.execFor(s, local, self)
	case *parser.ReturnStmt:
		value, err := in.evalExpr(s.Value, local)
		if err != nil {
			return nil, nil, err
		}
		return nil, returnSignal{value: value}, nil
	case *parser.BreakStmt:
		return nil, breakSignal{}, nil
	case *parser.ExprStmt:
		value, err := in.evalExpr(s.Expr, local)
		return value, nil, err
	default:
		return nil, nil, RuntimeError{Message: "unsupported statement", Span: stmtSpan(stmt)}
	}
}

type matchedBinding struct {
	name  string
	value Value
}

func (in *Interpreter) matchPattern(pattern parser.Pattern, value Value, local *env) ([]matchedBinding, bool, error) {
	switch p := pattern.(type) {
	case *parser.WildcardPattern:
		return nil, true, nil
	case *parser.BindingPattern:
		return []matchedBinding{{name: p.Name, value: value}}, true, nil
	case *parser.TypePattern:
		if !in.runtimeValueMatchesType(value, p.Target) {
			return nil, false, nil
		}
		if p.Name == "" || p.Name == "_" {
			return nil, true, nil
		}
		return []matchedBinding{{name: p.Name, value: value}}, true, nil
	case *parser.LiteralPattern:
		patternValue, err := in.matchLiteralValue(p.Value)
		if err != nil {
			return nil, false, err
		}
		equal, err := in.valuesEqual(value, patternValue, p.Span, local)
		if err != nil {
			return nil, false, err
		}
		return nil, equal, nil
	case *parser.TuplePattern:
		items, ok := tuplePatternValues(value)
		if !ok || len(items) != len(p.Elements) {
			return nil, false, nil
		}
		var bindings []matchedBinding
		for i, elem := range p.Elements {
			next, ok, err := in.matchPattern(elem, items[i], local)
			if err != nil || !ok {
				return nil, ok, err
			}
			bindings = append(bindings, next...)
		}
		return bindings, true, nil
	case *parser.ConstructorPattern:
		instanceValue, ok := value.(*instance)
		if !ok {
			return nil, false, nil
		}
		if instanceValue.class.Enum {
			caseName := ""
			switch len(p.Path) {
			case 1:
				caseName = p.Path[0]
			case 2:
				if p.Path[0] != instanceValue.class.Name {
					return nil, false, nil
				}
				caseName = p.Path[1]
			default:
				return nil, false, nil
			}
			if instanceValue.caseName != caseName {
				return nil, false, nil
			}
			var enumCase *parser.EnumCaseDecl
			for i := range instanceValue.class.Cases {
				if instanceValue.class.Cases[i].Name == caseName {
					enumCase = &instanceValue.class.Cases[i]
					break
				}
			}
			if enumCase == nil || len(p.Args) != len(enumCase.Fields) {
				return nil, false, nil
			}
			var bindings []matchedBinding
			for i, arg := range p.Args {
				fieldValue, ok := instanceValue.fields[enumCase.Fields[i].Name]
				if !ok {
					return nil, false, nil
				}
				next, ok, err := in.matchPattern(arg, fieldValue, local)
				if err != nil || !ok {
					return nil, ok, err
				}
				bindings = append(bindings, next...)
			}
			return bindings, true, nil
		}
		if len(p.Path) != 1 || p.Path[0] != instanceValue.class.Name {
			return nil, false, nil
		}
		items, _, ok := destructurableValues(value)
		if !ok || len(items) != len(p.Args) {
			return nil, false, nil
		}
		var bindings []matchedBinding
		for i, arg := range p.Args {
			next, ok, err := in.matchPattern(arg, items[i], local)
			if err != nil || !ok {
				return nil, ok, err
			}
			bindings = append(bindings, next...)
		}
		return bindings, true, nil
	default:
		return nil, false, RuntimeError{Message: "unsupported match pattern", Span: parser.Span{}}
	}
}

func (in *Interpreter) matchLiteralValue(expr parser.Expr) (Value, error) {
	switch e := expr.(type) {
	case *parser.IntegerLiteral:
		return strconv.ParseInt(e.Value, 10, 64)
	case *parser.FloatLiteral:
		return strconv.ParseFloat(e.Value, 64)
	case *parser.RuneLiteral:
		return decodeRuneLiteral(e.Value)
	case *parser.BoolLiteral:
		return e.Value, nil
	case *parser.StringLiteral:
		return e.Value, nil
	case *parser.UnitLiteral:
		return nil, nil
	default:
		return nil, RuntimeError{Message: "unsupported literal pattern", Span: exprSpan(expr)}
	}
}

func tuplePatternValues(value Value) ([]Value, bool) {
	tuple, ok := value.(*nativeTuple)
	if !ok {
		return nil, false
	}
	return tuple.items, true
}

func decodeRuneLiteral(raw string) (rune, error) {
	if len(raw) == 1 {
		return []rune(raw)[0], nil
	}
	switch raw {
	case `\n`:
		return '\n', nil
	case `\t`:
		return '\t', nil
	case `\r`:
		return '\r', nil
	case `\\`:
		return '\\', nil
	case `\'`:
		return '\'', nil
	case `\"`:
		return '"', nil
	default:
		return 0, fmt.Errorf("unsupported rune literal %q", raw)
	}
}

func (in *Interpreter) optionBindingValue(optionValue Value, local *env, span parser.Span) (bool, Value, error) {
	switch value := optionValue.(type) {
	case *nativeOption:
		if !value.set {
			return false, nil, nil
		}
		return true, value.value, nil
	case *instance:
		if value.class.Name != "Option" {
			return false, nil, RuntimeError{Message: "if binding requires Option[T]", Span: span}
		}
		setValue, err := in.invokeMethod(value, "isSet", nil, local, span)
		if err != nil {
			return false, nil, err
		}
		set, ok := setValue.(bool)
		if !ok || !set {
			return false, nil, nil
		}
		unwrapped, err := in.invokeMethod(value, "expect", nil, local, span)
		if err != nil {
			return false, nil, err
		}
		return true, unwrapped, nil
	default:
		return false, nil, RuntimeError{Message: "if binding requires Option[T]", Span: span}
	}
}

func (in *Interpreter) unwrappableBindingValue(sourceValue Value, local *env, span parser.Span) (bool, Value, error) {
	switch value := sourceValue.(type) {
	case *nativeOption:
		if !value.set {
			return false, nil, nil
		}
		return true, value.value, nil
	case *nativeResult:
		if !value.ok {
			return false, nil, nil
		}
		return true, value.value, nil
	case *nativeEither:
		if !value.rightSet {
			return false, nil, nil
		}
		return true, value.right, nil
	case *instance:
		if value.class.Name == "Option" {
			setValue, err := in.invokeMethod(value, "isSet", nil, local, span)
			if err != nil {
				return false, nil, err
			}
			set, ok := setValue.(bool)
			if !ok || !set {
				return false, nil, nil
			}
			unwrapped, err := in.invokeMethod(value, "expect", nil, local, span)
			if err != nil {
				return false, nil, err
			}
			return true, unwrapped, nil
		}
		if value.class.Name != "Result" && value.class.Name != "Either" {
			return false, nil, RuntimeError{Message: "unwrap binding requires Option[T], Result[T, E], or Either[L, R]", Span: span}
		}
		if value.class.Name == "Either" {
			isRightValue, err := in.invokeMethod(value, "isRight", nil, local, span)
			if err != nil {
				return false, nil, err
			}
			isRight, ok := isRightValue.(bool)
			if !ok {
				return false, nil, RuntimeError{Message: "Either.isRight must return Bool", Span: span}
			}
			if !isRight {
				return false, nil, nil
			}
			unwrapped, err := in.invokeMethod(value, "expectRight", nil, local, span)
			if err != nil {
				return false, nil, err
			}
			return true, unwrapped, nil
		}
		isOkValue, err := in.invokeMethod(value, "isOk", nil, local, span)
		if err != nil {
			return false, nil, err
		}
		isOk, ok := isOkValue.(bool)
		if !ok {
			return false, nil, RuntimeError{Message: value.class.Name + ".isOk must return Bool", Span: span}
		}
		if !isOk {
			return false, nil, nil
		}
		unwrapped, err := in.invokeMethod(value, "expect", nil, local, span)
		if err != nil {
			return false, nil, err
		}
		return true, unwrapped, nil
	default:
		return false, nil, RuntimeError{Message: "unwrap binding requires Option[T], Result[T, E], or Either[L, R]", Span: span}
	}
}

func (in *Interpreter) destructureBoundValue(bindings []parser.Binding, value Value, span parser.Span) ([]Value, error) {
	if len(bindings) <= 1 {
		return []Value{value}, nil
	}
	items, kind, ok := destructurableValues(value)
	if !ok {
		return nil, RuntimeError{Message: fmt.Sprintf("if binding expects %d values, got 1", len(bindings)), Span: span}
	}
	if len(items) != len(bindings) {
		return nil, RuntimeError{Message: fmt.Sprintf("if binding expects %d %s values, got %d", len(bindings), kind, len(items)), Span: span}
	}
	return append([]Value(nil), items...), nil
}

func (in *Interpreter) execFor(stmt *parser.ForStmt, local *env, self *instance) (Value, any, error) {
	if stmt.YieldBody != nil {
		var yielded []Value
		signal, err := in.execForBindings(stmt.Bindings, 0, local, self, func(loopEnv *env) (any, error) {
			value, signal, err := in.evalYieldBody(stmt.YieldBody, loopEnv, self)
			if err != nil {
				return nil, err
			}
			if signal == nil {
				yielded = append(yielded, value)
			}
			return signal, nil
		})
		if err != nil {
			return nil, nil, err
		}
		switch signal.(type) {
		case nil:
			return &nativeList{items: yielded}, nil, nil
		case breakSignal:
			return &nativeList{items: yielded}, nil, nil
		default:
			return nil, signal, nil
		}
	}
	signal, err := in.execForBindings(stmt.Bindings, 0, local, self, func(loopEnv *env) (any, error) {
		_, signal, err := in.execBlock(stmt.Body, loopEnv, self)
		return signal, err
	})
	if err != nil {
		return nil, nil, err
	}
	switch signal.(type) {
	case nil, breakSignal:
		return nil, nil, nil
	default:
		return nil, signal, nil
	}
}

func (in *Interpreter) execWhile(stmt *parser.WhileStmt, local *env, self *instance) (Value, any, error) {
	for {
		condValue, err := in.evalExpr(stmt.Condition, local)
		if err != nil {
			return nil, nil, err
		}
		cond, ok := condValue.(bool)
		if !ok {
			return nil, nil, RuntimeError{Message: "while condition must be Bool", Span: exprSpan(stmt.Condition)}
		}
		if !cond {
			return nil, nil, nil
		}
		_, signal, err := in.execBlock(stmt.Body, local, self)
		if err != nil {
			return nil, nil, err
		}
		switch signal.(type) {
		case nil:
		case breakSignal:
			return nil, nil, nil
		default:
			return nil, signal, nil
		}
	}
}

func (in *Interpreter) execForBindings(bindings []parser.ForBinding, index int, local *env, self *instance, body func(*env) (any, error)) (any, error) {
	if index == len(bindings) {
		return body(local)
	}
	binding := bindings[index]
	if binding.Iterable == nil {
		values, err := in.bindingValues(binding.Bindings, binding.Values, local, binding.Span)
		if err != nil {
			return nil, err
		}
		loopEnv := newEnv(local)
		for i, part := range binding.Bindings {
			if part.Name == "_" {
				continue
			}
			loopEnv.define(part.Name, in.coerceValueForBinding(part.Type, values[i]), part.Mutable)
		}
		signal, err := in.execForBindings(bindings, index+1, loopEnv, self, body)
		if err != nil {
			return nil, err
		}
		return signal, nil
	}
	iterable, err := in.evalExpr(binding.Iterable, local)
	if err != nil {
		return nil, err
	}
	items, ok := in.iterableToSlice(iterable, local, binding.Span)
	if !ok {
		return nil, RuntimeError{Message: "for loop expects iterable list value", Span: binding.Span}
	}
	for _, item := range items {
		loopEnv := newEnv(local)
		boundValues, err := in.destructureBoundValue(binding.Bindings, item, binding.Span)
		if err != nil {
			return nil, err
		}
		for i, part := range binding.Bindings {
			if part.Name == "_" {
				continue
			}
			loopEnv.define(part.Name, in.coerceValueForBinding(part.Type, boundValues[i]), false)
		}
		signal, err := in.execForBindings(bindings, index+1, loopEnv, self, body)
		if err != nil {
			return nil, err
		}
		switch signal.(type) {
		case nil:
		case breakSignal:
			return breakSignal{}, nil
		default:
			return signal, nil
		}
	}
	return nil, nil
}

func (in *Interpreter) evalYieldBody(block *parser.BlockStmt, local *env, self *instance) (Value, any, error) {
	if block == nil || len(block.Statements) == 0 {
		return nil, nil, RuntimeError{Message: "yield body must end with a value-producing statement", Span: parser.Span{}}
	}
	for i := 0; i < len(block.Statements)-1; i++ {
		_, signal, err := in.execStmt(block.Statements[i], local, self)
		if err != nil || signal != nil {
			return nil, signal, err
		}
	}
	last := block.Statements[len(block.Statements)-1]
	return in.evalStmtValue(last, local, self, "yield body must end with a value-producing statement")
}

func (in *Interpreter) evalBlockValue(block *parser.BlockStmt, local *env, self *instance, message string) (Value, any, error) {
	if block == nil || len(block.Statements) == 0 {
		return nil, nil, RuntimeError{Message: message, Span: parser.Span{}}
	}
	blockEnv := newEnv(local)
	for i := 0; i < len(block.Statements)-1; i++ {
		_, signal, err := in.execStmt(block.Statements[i], blockEnv, self)
		if err != nil || signal != nil {
			return nil, signal, err
		}
	}
	last := block.Statements[len(block.Statements)-1]
	return in.evalStmtValue(last, blockEnv, self, message)
}

func (in *Interpreter) evalStmtValue(stmt parser.Statement, local *env, self *instance, message string) (Value, any, error) {
	switch s := stmt.(type) {
	case *parser.ExprStmt:
		value, err := in.evalExpr(s.Expr, local)
		return value, nil, err
	case *parser.IfStmt:
		return in.evalIfStmtValue(s, local, self, message)
	case *parser.MatchStmt:
		return in.evalMatchStmtValue(s, local, self, message)
	case *parser.WhileStmt:
		return nil, nil, RuntimeError{Message: message, Span: stmtSpan(stmt)}
	case *parser.ForStmt:
		if s.YieldBody != nil {
			return in.execFor(s, local, self)
		}
		return nil, nil, RuntimeError{Message: message, Span: stmtSpan(stmt)}
	case *parser.ReturnStmt:
		value, signal, err := in.execStmt(s, local, self)
		return value, signal, err
	default:
		return nil, nil, RuntimeError{Message: message, Span: stmtSpan(stmt)}
	}
}

func (in *Interpreter) execUnwrapBinding(bindings []parser.Binding, expr parser.Expr, span parser.Span, local *env) (bool, Value, error) {
	sourceValue, err := in.evalExpr(expr, local)
	if err != nil {
		return false, nil, err
	}
	ok, unwrapped, err := in.unwrappableBindingValue(sourceValue, local, exprSpan(expr))
	if err != nil {
		return false, nil, err
	}
	if !ok {
		return false, sourceValue, nil
	}
	values, err := in.destructureBoundValue(bindings, unwrapped, span)
	if err != nil {
		return false, nil, err
	}
	for i, binding := range bindings {
		if binding.Name == "_" {
			continue
		}
		var value Value
		if i < len(values) {
			value = in.coerceValueForBinding(binding.Type, values[i])
		}
		local.define(binding.Name, value, false)
	}
	return true, nil, nil
}

func (in *Interpreter) evalIfStmtValue(s *parser.IfStmt, local *env, self *instance, message string) (Value, any, error) {
	if s.BindingValue != nil {
		optionValue, err := in.evalExpr(s.BindingValue, local)
		if err != nil {
			return nil, nil, err
		}
		set, value, err := in.optionBindingValue(optionValue, local, exprSpan(s.BindingValue))
		if err != nil {
			return nil, nil, err
		}
		if set {
			thenEnv := newEnv(local)
			boundValues, err := in.destructureBoundValue(s.Bindings, value, s.Span)
			if err != nil {
				return nil, nil, err
			}
			for i, binding := range s.Bindings {
				if binding.Name == "_" {
					continue
				}
				thenEnv.define(binding.Name, in.coerceValueForBinding(binding.Type, boundValues[i]), false)
			}
			return in.evalBlockValue(s.Then, thenEnv, self, message)
		}
	} else {
		cond, err := in.evalExpr(s.Condition, local)
		if err != nil {
			return nil, nil, err
		}
		truthy, err := asBool(cond, exprSpan(s.Condition))
		if err != nil {
			return nil, nil, err
		}
		if truthy {
			return in.evalBlockValue(s.Then, local, self, message)
		}
	}
	if s.ElseIf != nil {
		return in.evalIfStmtValue(s.ElseIf, local, self, message)
	}
	if s.Else != nil {
		return in.evalBlockValue(s.Else, local, self, message)
	}
	return nil, nil, RuntimeError{Message: message, Span: s.Span}
}

func (in *Interpreter) evalMatchStmtValue(s *parser.MatchStmt, local *env, self *instance, message string) (Value, any, error) {
	value, err := in.evalExpr(s.Value, local)
	if err != nil {
		return nil, nil, err
	}
	for _, matchCase := range s.Cases {
		bindings, ok, err := in.matchPattern(matchCase.Pattern, value, local)
		if err != nil {
			return nil, nil, err
		}
		if !ok {
			continue
		}
		caseEnv := newEnv(local)
		for _, binding := range bindings {
			caseEnv.define(binding.name, binding.value, false)
		}
		if matchCase.Guard != nil {
			guardValue, err := in.evalExpr(matchCase.Guard, caseEnv)
			if err != nil {
				return nil, nil, err
			}
			truthy, err := asBool(guardValue, exprSpan(matchCase.Guard))
			if err != nil {
				return nil, nil, err
			}
			if !truthy {
				continue
			}
		}
		if matchCase.Body != nil {
			value, signal, err := in.evalBlockValue(matchCase.Body, caseEnv, self, message)
			if err != nil {
				return nil, nil, err
			}
			if !s.Partial {
				return value, signal, nil
			}
			if signal != nil {
				return nil, signal, nil
			}
			wrapped, err := in.constructStdlibOption(value, true, local, s.Span)
			return wrapped, nil, err
		}
		if matchCase.Expr != nil {
			value, err := in.evalExpr(matchCase.Expr, caseEnv)
			if err != nil {
				return nil, nil, err
			}
			if !s.Partial {
				return value, nil, nil
			}
			wrapped, err := in.constructStdlibOption(value, true, local, s.Span)
			return wrapped, nil, err
		}
		return nil, nil, RuntimeError{Message: message, Span: matchCase.Span}
	}
	if s.Partial {
		value, err := in.constructStdlibOption(nil, false, local, s.Span)
		return value, nil, err
	}
	return nil, nil, RuntimeError{Message: "non-exhaustive match statement used as value", Span: s.Span}
}

func (in *Interpreter) assign(target parser.Expr, operator string, value Value, local *env) error {
	switch t := target.(type) {
	case *parser.Identifier:
		current, ok := local.resolve(t.Name)
		if !ok {
			return RuntimeError{Message: "undefined name '" + t.Name + "'", Span: t.Span}
		}
		if !current.mutable() {
			return RuntimeError{Message: "cannot assign to immutable binding '" + t.Name + "'", Span: t.Span}
		}
		if operator == "=" {
			return RuntimeError{Message: "use ':=' for mutable reassignment", Span: t.Span}
		}
		if operator != ":=" {
			updated, err := applyBinary(operator[:len(operator)-1], current.value(), value, t.Span)
			if err != nil {
				return err
			}
			value = updated
		}
		return current.set(value)
	case *parser.MemberExpr:
		receiver, err := in.evalExpr(t.Receiver, local)
		if err != nil {
			return err
		}
		obj, ok := receiver.(*instance)
		if !ok {
			return RuntimeError{Message: "member assignment expects class instance", Span: t.Span}
		}
		current := obj.fields[t.Name]
		if operator != "=" && operator != ":=" {
			updated, err := applyBinary(operator[:len(operator)-1], current, value, t.Span)
			if err != nil {
				return err
			}
			value = updated
		}
		obj.fields[t.Name] = value
		return nil
	case *parser.IndexExpr:
		receiver, err := in.evalExpr(t.Receiver, local)
		if err != nil {
			return err
		}
		if m, ok := receiver.(*nativeMap); ok {
			keyValue, err := in.evalExpr(t.Index, local)
			if err != nil {
				return err
			}
			if operator == "=" {
				return RuntimeError{Message: "use ':=' for mutable reassignment", Span: t.Span}
			}
			if operator != ":=" {
				return RuntimeError{Message: "compound assignment is not supported for map indexes", Span: t.Span}
			}
			key, err := nativeKey(keyValue, exprSpan(t.Index), local, in)
			if err != nil {
				return err
			}
			if _, exists := m.items[key]; !exists {
				m.order = append(m.order, key)
				m.keys[key] = keyValue
			}
			m.items[key] = value
			return nil
		}
		items, ok := indexedItems(receiver)
		if !ok {
			return RuntimeError{Message: "index assignment requires array-like value", Span: t.Span}
		}
		indexValue, err := in.evalExpr(t.Index, local)
		if err != nil {
			return err
		}
		index, ok := indexValue.(int64)
		if !ok {
			return RuntimeError{Message: "index expression must be Int", Span: exprSpan(t.Index)}
		}
		if index < 0 || index >= int64(len(items)) {
			return RuntimeError{Message: "index out of bounds", Span: t.Span}
		}
		if operator == "=" {
			return RuntimeError{Message: "use ':=' for mutable reassignment", Span: t.Span}
		}
		if operator != "=" && operator != ":=" {
			updated, err := applyBinary(operator[:len(operator)-1], items[index], value, t.Span)
			if err != nil {
				return err
			}
			value = updated
		}
		items[index] = value
		return nil
	default:
		return RuntimeError{Message: "invalid assignment target", Span: exprSpan(target)}
	}
}

func (in *Interpreter) evalExpr(expr parser.Expr, local *env) (Value, error) {
	switch e := expr.(type) {
	case *parser.Identifier:
		if thisScope, thisObj, ok := local.findThisScope(); ok && thisObj != nil {
			if binding, ok := local.getUntil(thisScope, e.Name); ok {
				if _, deferred := binding.value.(deferredValue); deferred {
					return nil, RuntimeError{Message: "binding '" + e.Name + "' is deferred and has not been assigned", Span: e.Span}
				}
				return binding.value, nil
			}
			if value, exists := thisObj.fields[e.Name]; exists {
				if _, deferred := value.(deferredValue); deferred {
					return nil, RuntimeError{Message: "field '" + e.Name + "' is deferred and has not been assigned", Span: e.Span}
				}
				return value, nil
			}
			if thisScope.parent != nil {
				if binding, ok := thisScope.parent.get(e.Name); ok {
					if _, deferred := binding.value.(deferredValue); deferred {
						return nil, RuntimeError{Message: "binding '" + e.Name + "' is deferred and has not been assigned", Span: e.Span}
					}
					return binding.value, nil
				}
			}
		} else if binding, ok := local.get(e.Name); ok {
			if _, deferred := binding.value.(deferredValue); deferred {
				return nil, RuntimeError{Message: "binding '" + e.Name + "' is deferred and has not been assigned", Span: e.Span}
			}
			return binding.value, nil
		}
		if _, ok := in.functions[e.Name]; ok {
			return functionRef{module: in, name: e.Name}, nil
		}
		if isImplicitOSMethod(e.Name) {
			return boundMethodRef{receiver: newNativeOS(), name: e.Name}, nil
		}
		switch e.Name {
		case "List", "Set", "Map", "Array":
			return builtinRef{name: e.Name}, nil
		case "OS":
			return newNativeOS(), nil
		}
		if _, ok := in.enumCases[e.Name]; ok {
			return builtinRef{name: e.Name}, nil
		}
		if _, ok := in.imports[e.Name]; ok {
			return moduleRef{name: e.Name}, nil
		}
		if _, ok := in.objects[e.Name]; ok {
			if slot, ok := local.get(e.Name); ok {
				return slot.value, nil
			}
		}
		if _, ok := in.classes[e.Name]; ok {
			return classRef{module: in, name: e.Name}, nil
		}
		return nil, RuntimeError{Message: "undefined name '" + e.Name + "'", Span: e.Span}
	case *parser.IntegerLiteral:
		n, err := strconv.ParseInt(e.Value, 10, 64)
		if err != nil {
			return nil, RuntimeError{Message: "invalid integer literal", Span: e.Span}
		}
		return n, nil
	case *parser.FloatLiteral:
		n, err := strconv.ParseFloat(e.Value, 64)
		if err != nil {
			return nil, RuntimeError{Message: "invalid float literal", Span: e.Span}
		}
		return n, nil
	case *parser.RuneLiteral:
		return []rune(e.Value)[0], nil
	case *parser.BoolLiteral:
		return e.Value, nil
	case *parser.StringLiteral:
		return e.Value, nil
	case *parser.UnitLiteral:
		return nil, nil
	case *parser.ListLiteral:
		items := make([]Value, len(e.Elements))
		for i, item := range e.Elements {
			value, err := in.evalExpr(item, local)
			if err != nil {
				return nil, err
			}
			items[i] = value
		}
		return &nativeList{items: items}, nil
	case *parser.TupleLiteral:
		items := make([]Value, len(e.Elements))
		for i, item := range e.Elements {
			value, err := in.evalExpr(item, local)
			if err != nil {
				return nil, err
			}
			items[i] = value
		}
		return &nativeTuple{items: items}, nil
	case *parser.IfExpr:
		cond, err := in.evalExpr(e.Condition, local)
		if err != nil {
			return nil, err
		}
		truthy, err := asBool(cond, exprSpan(e.Condition))
		if err != nil {
			return nil, err
		}
		if truthy {
			value, signal, err := in.evalBlockValue(e.Then, local, nil, "if expression branches must end with an expression")
			if err != nil {
				return nil, err
			}
			if signal != nil {
				return nil, RuntimeError{Message: "unexpected control flow in if expression", Span: e.Span}
			}
			return value, nil
		}
		value, signal, err := in.evalBlockValue(e.Else, local, nil, "if expression branches must end with an expression")
		if err != nil {
			return nil, err
		}
		if signal != nil {
			return nil, RuntimeError{Message: "unexpected control flow in if expression", Span: e.Span}
		}
		return value, nil
	case *parser.BlockExpr:
		value, signal, err := in.evalBlockValue(e.Body, local, nil, "block expression must end with an expression")
		if err != nil {
			return nil, err
		}
		if signal != nil {
			return nil, RuntimeError{Message: "unexpected control flow in block expression", Span: e.Span}
		}
		return value, nil
	case *parser.MatchExpr:
		value, err := in.evalExpr(e.Value, local)
		if err != nil {
			return nil, err
		}
		for _, matchCase := range e.Cases {
			bindings, ok, err := in.matchPattern(matchCase.Pattern, value, local)
			if err != nil {
				return nil, err
			}
			if !ok {
				continue
			}
			caseEnv := newEnv(local)
			for _, binding := range bindings {
				caseEnv.define(binding.name, binding.value, false)
			}
			if matchCase.Guard != nil {
				guardValue, err := in.evalExpr(matchCase.Guard, caseEnv)
				if err != nil {
					return nil, err
				}
				truthy, err := asBool(guardValue, exprSpan(matchCase.Guard))
				if err != nil {
					return nil, err
				}
				if !truthy {
					continue
				}
			}
			if matchCase.Body != nil {
				value, signal, err := in.evalBlockValue(matchCase.Body, caseEnv, nil, "match case must end with an expression")
				if err != nil {
					return nil, err
				}
				if signal != nil {
					return nil, RuntimeError{Message: "unexpected control flow in match expression", Span: e.Span}
				}
				if e.Partial {
					return in.constructStdlibOption(value, true, local, e.Span)
				}
				return value, nil
			}
			value, err := in.evalExpr(matchCase.Expr, caseEnv)
			if err != nil {
				return nil, err
			}
			if e.Partial {
				return in.constructStdlibOption(value, true, local, e.Span)
			}
			return value, nil
		}
		if e.Partial {
			return in.constructStdlibOption(nil, false, local, e.Span)
		}
		return nil, RuntimeError{Message: "non-exhaustive match expression", Span: e.Span}
	case *parser.ForYieldExpr:
		var yielded []Value
		signal, err := in.execForBindings(e.Bindings, 0, local, nil, func(loopEnv *env) (any, error) {
			value, signal, err := in.evalYieldBody(e.YieldBody, loopEnv, nil)
			if err != nil {
				return nil, err
			}
			if signal == nil {
				yielded = append(yielded, value)
			}
			return signal, nil
		})
		if err != nil {
			return nil, err
		}
		switch signal.(type) {
		case nil, breakSignal:
			return &nativeList{items: yielded}, nil
		default:
			return nil, RuntimeError{Message: "unexpected control flow in yield expression", Span: e.Span}
		}
	case *parser.GroupExpr:
		return in.evalExpr(e.Inner, local)
	case *parser.UnaryExpr:
		right, err := in.evalExpr(e.Right, local)
		if err != nil {
			return nil, err
		}
		switch e.Operator {
		case "!":
			b, err := asBool(right, e.Span)
			if err != nil {
				return nil, err
			}
			return !b, nil
		case "-":
			if obj, ok := right.(*instance); ok {
				return in.invokeMethod(obj, "-", nil, local, e.Span)
			}
			switch n := right.(type) {
			case int64:
				return -n, nil
			case float64:
				return -n, nil
			default:
				return nil, RuntimeError{Message: "operator - requires numeric operand", Span: e.Span}
			}
		case "~":
			if obj, ok := right.(*instance); ok {
				return in.invokeMethod(obj, "~", nil, local, e.Span)
			}
			return nil, RuntimeError{Message: "operator ~ requires an overloaded operand", Span: e.Span}
		default:
			return nil, RuntimeError{Message: "unsupported unary operator", Span: e.Span}
		}
	case *parser.BinaryExpr:
		left, err := in.evalExpr(e.Left, local)
		if err != nil {
			return nil, err
		}
		if e.Operator == "&&" {
			lb, err := asBool(left, e.Span)
			if err != nil {
				return nil, err
			}
			if !lb {
				return false, nil
			}
		}
		if e.Operator == "||" {
			lb, err := asBool(left, e.Span)
			if err != nil {
				return nil, err
			}
			if lb {
				return true, nil
			}
		}
		right, err := in.evalExpr(e.Right, local)
		if err != nil {
			return nil, err
		}
		if e.Operator == "==" || e.Operator == "!=" {
			equal, err := in.valuesEqual(left, right, e.Span, local)
			if err != nil {
				return nil, err
			}
			if e.Operator == "!=" {
				return !equal, nil
			}
			return equal, nil
		}
		if overloaded, ok, err := in.evalOverloadedBinary(e.Operator, left, e.Right, right, local, e.Span); ok {
			return overloaded, err
		}
		return applyBinary(e.Operator, left, right, e.Span)
	case *parser.IsExpr:
		left, err := in.evalExpr(e.Left, local)
		if err != nil {
			return nil, err
		}
		return in.runtimeValueMatchesType(left, e.Target), nil
	case *parser.CallExpr:
		return in.evalCall(e, local)
	case *parser.MemberExpr:
		if ident, ok := e.Receiver.(*parser.Identifier); ok {
			if value, ok, err := in.tryEnumCaseMemberFromIdentifier(local, ident.Name, e.Name, e.Span); ok || err != nil {
				return value, err
			}
		}
		receiver, err := in.evalExpr(e.Receiver, local)
		if err != nil {
			return nil, err
		}
		return in.evalMember(receiver, e)
	case *parser.IndexExpr:
		receiver, err := in.evalExpr(e.Receiver, local)
		if err != nil {
			return nil, err
		}
		indexValue, err := in.evalExpr(e.Index, local)
		if err != nil {
			return nil, err
		}
		if obj, ok := receiver.(*instance); ok {
			return in.invokeMethod(obj, "[]", []Value{indexValue}, local, e.Span)
		}
		items, ok := indexedItems(receiver)
		if !ok {
			if m, ok := receiver.(*nativeMap); ok {
				key, err := nativeKey(indexValue, e.Span, local, in)
				if err != nil {
					return nil, err
				}
				value, exists := m.items[key]
				if !exists {
					return in.constructStdlibOption(nil, false, local, e.Span)
				}
				return in.constructStdlibOption(value, true, local, e.Span)
			}
			return nil, RuntimeError{Message: "indexing requires Array, List, Map, or operator []", Span: e.Span}
		}
		index, ok := indexValue.(int64)
		if !ok {
			return nil, RuntimeError{Message: "index expression must be Int", Span: exprSpan(e.Index)}
		}
		if index < 0 || index >= int64(len(items)) {
			return nil, RuntimeError{Message: "index out of bounds", Span: e.Span}
		}
		return items[index], nil
	case *parser.RecordUpdateExpr:
		receiver, err := in.evalExpr(e.Receiver, local)
		if err != nil {
			return nil, err
		}
		record, ok := receiver.(*instance)
		if !ok || record.class.Object || record.class.Enum {
			return nil, RuntimeError{Message: "update requires a record or class value", Span: e.Span}
		}
		if !record.class.Record {
			for _, field := range record.class.Fields {
				if field.Private {
					return nil, RuntimeError{Message: "class update requires a class without private fields", Span: e.Span}
				}
			}
		}
		copyFields := make(map[string]Value, len(record.fields))
		for name, value := range record.fields {
			copyFields[name] = value
		}
		for _, update := range e.Updates {
			value, err := in.evalExpr(update.Value, local)
			if err != nil {
				return nil, err
			}
			copyFields[update.Name] = value
		}
		return &instance{class: record.class, fields: copyFields}, nil
	case *parser.AnonymousRecordExpr:
		if len(e.Values) > 0 {
			return nil, RuntimeError{Message: "positional record(...) requires a known anonymous record shape", Span: e.Span}
		}
		fields := make(map[string]Value, len(e.Fields))
		order := make([]string, 0, len(e.Fields))
		for _, field := range e.Fields {
			value, err := in.evalExpr(field.Value, local)
			if err != nil {
				return nil, err
			}
			if _, exists := fields[field.Name]; !exists {
				order = append(order, field.Name)
			}
			fields[field.Name] = value
		}
		return &nativeRecord{fields: fields, order: order}, nil
	case *parser.AnonymousInterfaceExpr:
		class := &parser.ClassDecl{
			Name:       fmt.Sprintf("__anon_iface_%d_%d", e.Span.Start.Line, e.Span.Start.Column),
			Implements: e.Interfaces,
			Methods:    e.Methods,
			Span:       e.Span,
		}
		return in.construct(class, nil, local)
	case *parser.LambdaExpr:
		params := make([]string, len(e.Parameters))
		for i, param := range e.Parameters {
			params[i] = param.Name
		}
		return &closure{params: params, tupleDestructuring: len(e.Parameters) > 1, body: e.Body, blockBody: e.BlockBody, env: local}, nil
	case *parser.PlaceholderExpr:
		return nil, RuntimeError{Message: "placeholder is not supported here", Span: e.Span}
	default:
		return nil, RuntimeError{Message: "unsupported expression", Span: exprSpan(expr)}
	}
}

func (in *Interpreter) evalCall(call *parser.CallExpr, local *env) (Value, error) {
	if ident, ok := call.Callee.(*parser.Identifier); ok && ident.Name == "init" {
		ctorFlag, ok := local.get("__constructor__")
		if !ok || ctorFlag.value != true {
			return nil, RuntimeError{Message: "'init(...)' is only valid inside constructors", Span: call.Span}
		}
		thisSlot, ok := local.get("this")
		if !ok {
			return nil, RuntimeError{Message: "missing this receiver", Span: call.Span}
		}
		obj, ok := thisSlot.value.(*instance)
		if !ok {
			return nil, RuntimeError{Message: "invalid this receiver", Span: call.Span}
		}
		args := make([]namedValueArg, len(call.Args))
		for i, arg := range call.Args {
			value, err := in.evalExpr(arg.Value, local)
			if err != nil {
				return nil, err
			}
			args[i] = namedValueArg{Name: arg.Name, Value: value, Span: arg.Span}
		}
		ordered := namedArgValues(args)
		if hasNamedParserArgs(call.Args) {
			reordered, err := reorderConstructorValueArgs(obj.class, args, call.Span)
			if err != nil {
				return nil, err
			}
			ordered = reordered
		}
		return nil, in.constructInto(obj, obj.class, ordered, local, call.Span)
	}
	if member, ok := call.Callee.(*parser.MemberExpr); ok {
		return in.evalMethodCall(member, call.Args, local)
	}
	callee, err := in.evalExpr(call.Callee, local)
	if err != nil {
		return nil, err
	}
	if fn, ok := callee.(builtinRef); ok {
		return in.callBuiltin(fn.name, call.Args, nil, local, call.Span)
	}
	if fn, ok := callee.(functionRef); ok {
		decl, ok := fn.module.functions[fn.name]
		if !ok {
			return nil, RuntimeError{Message: "undefined function '" + fn.name + "'", Span: call.Span}
		}
		ordered, err := in.evalArgsWithParams(decl.Parameters, call.Args, local, call.Span, "function '"+fn.name+"'")
		if err != nil {
			return nil, err
		}
		return fn.module.callFunctionByName(fn.name, ordered, local)
	}
	if method, ok := callee.(boundMethodRef); ok {
		ordered, err := in.evalBoundMethodArgs(method.receiver, method.name, call.Args, local, call.Span)
		if err != nil {
			return nil, err
		}
		return in.invokeBoundMethod(method.receiver, method.name, ordered, local, call.Span)
	}
	if fn, ok := callee.(classRef); ok && len(call.Args) == 1 {
		class, ok := fn.module.classes[fn.name]
		if !ok {
			return nil, RuntimeError{Message: "undefined class '" + fn.name + "'", Span: call.Span}
		}
		if recordExpr, ok := call.Args[0].Value.(*parser.AnonymousRecordExpr); ok && len(recordExpr.Values) > 0 {
			required, optional, shapeOK := fn.module.visibleClassShape(class)
			exposed := append(append([]parser.FieldDecl{}, required...), optional...)
			if shapeOK && len(recordExpr.Values) >= len(required) && len(recordExpr.Values) <= len(exposed) {
				args := make([]Value, len(recordExpr.Values))
				matched := true
				for i, field := range exposed {
					if i >= len(recordExpr.Values) {
						break
					}
					value, err := in.evalExprWithTypeRef(recordExpr.Values[i], field.Type, local)
					if err != nil || !in.runtimeValueMatchesType(value, field.Type) {
						matched = false
						break
					}
					args[i] = value
				}
				if matched {
					recordFields := make(map[string]Value, len(recordExpr.Values))
					for i, field := range exposed[:len(recordExpr.Values)] {
						recordFields[field.Name] = args[i]
					}
					return fn.module.construct(class, []Value{&nativeRecord{fields: recordFields}}, local)
				}
			}
		}
	}
	args := make([]namedValueArg, len(call.Args))
	for i, arg := range call.Args {
		value, err := in.evalExpr(arg.Value, local)
		if err != nil {
			return nil, err
		}
		args[i] = namedValueArg{Name: arg.Name, Value: value, Span: arg.Span}
	}
	switch fn := callee.(type) {
	case classRef:
		class, ok := fn.module.classes[fn.name]
		if !ok {
			return nil, RuntimeError{Message: "undefined class '" + fn.name + "'", Span: call.Span}
		}
		ordered := namedArgValues(args)
		if len(ordered) == 1 {
			if record, ok := ordered[0].(*nativeRecord); ok {
				if !fn.module.recordMatchesVisibleClassShape(class, record) {
					return nil, RuntimeError{Message: "class/record '" + class.Name + "' requires an anonymous record with exactly matching field names and types", Span: call.Span}
				}
				return fn.module.construct(class, ordered, local)
			}
		}
		if class.Object {
			return nil, RuntimeError{Message: "object '" + fn.name + "' is not callable", Span: call.Span}
		}
		if hasNamedParserArgs(call.Args) {
			reordered, err := reorderConstructorValueArgs(class, args, call.Span)
			if err != nil {
				return nil, err
			}
			ordered = reordered
		}
		return fn.module.construct(class, ordered, local)
	case enumCaseRef:
		class, ok := fn.module.classes[fn.enumName]
		if !ok {
			return nil, RuntimeError{Message: "undefined enum '" + fn.enumName + "'", Span: call.Span}
		}
		for _, enumCase := range class.Cases {
			if enumCase.Name != fn.caseName {
				continue
			}
			ordered := namedArgValues(args)
			if hasNamedParserArgs(call.Args) {
				params := make([]parser.Parameter, len(enumCase.Fields))
				for i, field := range enumCase.Fields {
					params[i] = parser.Parameter{Name: field.Name, Type: field.Type, Span: field.Span}
				}
				reordered, err := reorderNamedValueArgs(params, args, call.Span, "enum case '"+fn.caseName+"'")
				if err != nil {
					return nil, err
				}
				ordered = reordered
			}
			return fn.module.constructEnumCase(class, enumCase, ordered, local, call.Span)
		}
		return nil, RuntimeError{Message: "unknown enum case '" + fn.caseName + "'", Span: call.Span}
	case *closure:
		if hasNamedParserArgs(call.Args) {
			return nil, RuntimeError{Message: "named arguments require a direct function, method, or constructor call", Span: call.Span}
		}
		return in.callClosure(fn, namedArgValues(args))
	case *instance:
		if fn.class != nil && fn.class.Object {
			return nil, RuntimeError{Message: "object '" + fn.class.Name + "' is not callable", Span: call.Span}
		}
		if fn.class != nil {
			return nil, RuntimeError{Message: "value of type '" + fn.class.Name + "' is not callable", Span: call.Span}
		}
		return nil, RuntimeError{Message: "value is not callable", Span: call.Span}
	default:
		return nil, RuntimeError{Message: "value is not callable", Span: call.Span}
	}
}

func (in *Interpreter) callClosure(fn *closure, args []Value) (Value, error) {
	if !acceptsClosureArgCount(fn, len(args)) {
		return nil, RuntimeError{Message: fmt.Sprintf("lambda expects %s args, got %d", expectedClosureArgs(fn), len(args)), Span: parser.Span{}}
	}
	local := newEnv(fn.env)
	boundArgs := args
	if fn.tupleDestructuring && len(args) == 1 && len(fn.params) > 1 {
		items, kind, ok := destructurableValues(args[0])
		if !ok || len(items) != len(fn.params) {
			count := 1
			if ok {
				count = len(items)
			}
			return nil, RuntimeError{Message: fmt.Sprintf("lambda expects %d %s values, got %d", len(fn.params), kindOrTuple(kind), count), Span: parser.Span{}}
		}
		boundArgs = items
	}
	for i, param := range fn.params {
		if fn.variadic && i == len(fn.params)-1 {
			if param != "_" {
				local.define(param, &nativeList{items: append([]Value{}, boundArgs[i:]...)}, false)
			}
			break
		}
		if param != "_" {
			local.define(param, boundArgs[i], false)
		}
	}
	if fn.body != nil {
		value, err := in.evalExpr(fn.body, local)
		if err != nil {
			return nil, err
		}
		if isUnitTypeRef(fn.returnType) {
			return nil, nil
		}
		return in.coerceValueForTypeRef(fn.returnType, value), nil
	}
	if fn.blockBody != nil {
		value, signal, err := in.execBlock(fn.blockBody, local, nil)
		if err != nil {
			return nil, err
		}
		if ret, ok := signal.(returnSignal); ok {
			return in.coerceValueForTypeRef(fn.returnType, ret.value), nil
		}
		if isUnitTypeRef(fn.returnType) {
			return nil, nil
		}
		return in.coerceValueForTypeRef(fn.returnType, value), nil
	}
	return nil, nil
}

func (in *Interpreter) evalMethodCall(member *parser.MemberExpr, argExprs []parser.CallArg, local *env) (Value, error) {
	receiver, err := in.evalExpr(member.Receiver, local)
	if err != nil {
		return nil, err
	}
	if builtin, ok := receiver.(builtinRef); ok && builtin.name == "Array" && member.Name == "ofLength" {
		if len(argExprs) != 1 {
			return nil, RuntimeError{Message: fmt.Sprintf("Array.ofLength expects 1 argument, got %d", len(argExprs)), Span: member.Span}
		}
		lengthValue, err := in.evalExpr(argExprs[0].Value, local)
		if err != nil {
			return nil, err
		}
		length, ok := lengthValue.(int64)
		if !ok {
			return nil, RuntimeError{Message: "Array.ofLength expects Int length", Span: exprSpan(argExprs[0].Value)}
		}
		if length < 0 {
			return nil, RuntimeError{Message: "Array.ofLength length must be non-negative", Span: exprSpan(argExprs[0].Value)}
		}
		return &nativeArray{items: make([]Value, int(length))}, nil
	}
	if modRef, ok := receiver.(moduleRef); ok {
		mod, ok := in.imports[modRef.name]
		if !ok {
			return nil, RuntimeError{Message: "unknown module '" + modRef.name + "'", Span: member.Span}
		}
		if decl, ok := mod.functions[member.Name]; ok {
			ordered, err := in.evalArgsWithParams(decl.Parameters, argExprs, local, member.Span, "function '"+member.Name+"'")
			if err != nil {
				return nil, err
			}
			return mod.callFunctionByName(member.Name, ordered, mod.globals)
		}
		args := make([]namedValueArg, len(argExprs))
		for i, arg := range argExprs {
			value, err := in.evalExpr(arg.Value, local)
			if err != nil {
				return nil, err
			}
			args[i] = namedValueArg{Name: arg.Name, Value: value, Span: arg.Span}
		}
		ordered := namedArgValues(args)
		if class, ok := mod.classes[member.Name]; ok {
			if hasNamedParserArgs(argExprs) {
				reordered, err := reorderConstructorValueArgs(class, args, member.Span)
				if err != nil {
					return nil, err
				}
				ordered = reordered
			}
			return mod.construct(class, ordered, mod.globals)
		}
		if _, ok := mod.objects[member.Name]; ok {
			value, ok := mod.globals.get(member.Name)
			if !ok {
				return nil, RuntimeError{Message: "undefined object '" + member.Name + "'", Span: member.Span}
			}
			argsOnly := make([]Value, len(ordered))
			for i, arg := range ordered {
				argsOnly[i] = arg
			}
			return mod.invokeCallableValue(value.value, argsOnly, mod.globals, member.Span)
		}
		return nil, RuntimeError{Message: "unknown member '" + member.Name + "'", Span: member.Span}
	}
	if class, ok := receiver.(classRef); ok {
		decl, found := class.module.classes[class.name]
		if !found {
			return nil, RuntimeError{Message: "unknown class '" + class.name + "'", Span: member.Span}
		}
		if decl.Enum {
			for _, enumCase := range decl.Cases {
				if enumCase.Name != member.Name {
					continue
				}
				params := make([]parser.Parameter, len(enumCase.Fields))
				for i, field := range enumCase.Fields {
					params[i] = parser.Parameter{Name: field.Name, Type: field.Type, Span: field.Span}
				}
				ordered, err := in.evalArgsWithParams(params, argExprs, local, member.Span, "enum case '"+member.Name+"'")
				if err != nil {
					return nil, err
				}
				return class.module.constructEnumCase(decl, enumCase, ordered, local, member.Span)
			}
		}
	}
	if method, ok := in.nativeMethodDescriptor(receiver, member.Name); ok {
		ordered, err := in.evalArgsWithParams(method.Parameters, argExprs, local, member.Span, "method '"+member.Name+"'")
		if err != nil {
			return nil, err
		}
		named := make([]namedValueArg, len(ordered))
		for i, value := range ordered {
			span := member.Span
			if i < len(argExprs) {
				span = argExprs[i].Span
			}
			named[i] = namedValueArg{Value: value, Span: span}
		}
		if native, ok := in.callNativeMethod(receiver, member.Name, named, local, member.Span); ok {
			return native.value, native.err
		}
	}
	args := make([]namedValueArg, len(argExprs))
	for i, arg := range argExprs {
		value, err := in.evalExpr(arg.Value, local)
		if err != nil {
			return nil, err
		}
		args[i] = namedValueArg{Name: arg.Name, Value: value, Span: arg.Span}
	}
	if native, ok := in.callNativeMethod(receiver, member.Name, args, local, member.Span); ok {
		return native.value, native.err
	}
	obj, ok := receiver.(*instance)
	if !ok {
		return nil, RuntimeError{Message: "member call requires class instance", Span: member.Span}
	}
	methods := instanceMethods(obj)
	if len(methods) > 0 {
		var candidates []*parser.MethodDecl
		for _, method := range methods {
			if method.Name == member.Name {
				candidates = append(candidates, method)
			}
		}
		if len(candidates) == 1 {
			ordered, err := in.evalArgsWithParams(candidates[0].Parameters, argExprs, local, member.Span, "method '"+member.Name+"'")
			if err != nil {
				return nil, err
			}
			return in.callMethod(obj, candidates[0], ordered, local)
		}
	}
	if hasNamedParserArgs(argExprs) {
		var candidates []*parser.MethodDecl
		for _, method := range methods {
			if method.Name == member.Name {
				candidates = append(candidates, method)
			}
		}
		if len(candidates) == 1 {
			ordered, err := reorderNamedValueArgs(candidates[0].Parameters, args, member.Span, "method '"+member.Name+"'")
			if err != nil {
				return nil, err
			}
			return in.callMethod(obj, candidates[0], ordered, local)
		}
	}
	var firstErr error
	var fallbackMethod *parser.MethodDecl
	var fallbackArgs []Value
	for _, method := range methods {
		if method.Name != member.Name {
			continue
		}
		ordered := namedArgValues(args)
		if hasNamedParserArgs(argExprs) {
			reordered, err := reorderNamedValueArgs(method.Parameters, args, member.Span, "method '"+member.Name+"'")
			if err != nil {
				if firstErr == nil {
					firstErr = err
				}
				continue
			}
			ordered = reordered
			if fallbackMethod == nil {
				fallbackMethod = method
				fallbackArgs = ordered
			}
		}
		if in.runtimeMethodMatches(method, ordered) {
			return in.callMethod(obj, method, ordered, local)
		}
	}
	if fallbackMethod != nil {
		return in.callMethod(obj, fallbackMethod, fallbackArgs, local)
	}
	if firstErr != nil {
		return nil, firstErr
	}
	defaultMethods := in.defaultInterfaceMethodsByName(obj, member.Name)
	if len(defaultMethods) > 0 {
		if len(defaultMethods) == 1 {
			ordered, err := in.evalArgsWithParams(defaultMethods[0].Parameters, argExprs, local, member.Span, "method '"+member.Name+"'")
			if err != nil {
				return nil, err
			}
			return in.callInterfaceMethod(obj, defaultMethods[0], ordered, local)
		}
		if hasNamedParserArgs(argExprs) {
			orderedExprs, err := reorderNamedParserArgs(defaultMethods[0].Parameters, argExprs, member.Span, "method '"+member.Name+"'")
			if err == nil {
				ordered := make([]Value, len(orderedExprs))
				for i, expr := range orderedExprs {
					value, evalErr := in.evalExpr(expr, local)
					if evalErr != nil {
						return nil, evalErr
					}
					ordered[i] = value
				}
				if method, ok := in.findDefaultInterfaceMethod(obj, member.Name, ordered); ok {
					return in.callInterfaceMethod(obj, method, ordered, local)
				}
			}
		} else {
			ordered := namedArgValues(args)
			if method, ok := in.findDefaultInterfaceMethod(obj, member.Name, ordered); ok {
				return in.callInterfaceMethod(obj, method, ordered, local)
			}
		}
	}
	return nil, RuntimeError{Message: "unknown method '" + member.Name + "'", Span: member.Span}
}

func (in *Interpreter) evalBoundMethodArgs(receiver Value, methodName string, argExprs []parser.CallArg, local *env, span parser.Span) ([]Value, error) {
	if descriptor, ok := in.nativeMethodDescriptor(receiver, methodName); ok {
		return in.evalArgsWithParams(descriptor.Parameters, argExprs, local, span, "method '"+methodName+"'")
	}
	obj, ok := receiver.(*instance)
	if !ok {
		return nil, RuntimeError{Message: "unknown method '" + methodName + "'", Span: span}
	}
	methods := instanceMethods(obj)
	var candidates []*parser.MethodDecl
	for _, method := range methods {
		if method.Name == methodName {
			candidates = append(candidates, method)
		}
	}
	if len(candidates) == 1 {
		return in.evalArgsWithParams(candidates[0].Parameters, argExprs, local, span, "method '"+methodName+"'")
	}
	args := make([]namedValueArg, len(argExprs))
	for i, arg := range argExprs {
		value, err := in.evalExpr(arg.Value, local)
		if err != nil {
			return nil, err
		}
		args[i] = namedValueArg{Name: arg.Name, Value: value, Span: arg.Span}
	}
	if hasNamedParserArgs(argExprs) {
		var firstErr error
		for _, method := range candidates {
			reordered, err := reorderNamedValueArgs(method.Parameters, args, span, "method '"+methodName+"'")
			if err != nil {
				if firstErr == nil {
					firstErr = err
				}
				continue
			}
			return reordered, nil
		}
		if firstErr != nil {
			return nil, firstErr
		}
	}
	return namedArgValues(args), nil
}

func (in *Interpreter) invokeBoundMethod(receiver Value, methodName string, args []Value, local *env, span parser.Span) (Value, error) {
	if descriptor, ok := in.nativeMethodDescriptor(receiver, methodName); ok {
		named := make([]namedValueArg, len(args))
		for i, arg := range args {
			named[i] = namedValueArg{Value: arg, Span: span}
		}
		if !acceptsArgCount(descriptor.Parameters, len(args)) {
			return nil, RuntimeError{Message: fmt.Sprintf("method '%s' expects %s arguments, got %d", methodName, expectedCallableArgs(descriptor.Parameters), len(args)), Span: span}
		}
		if native, ok := in.callNativeMethod(receiver, methodName, named, local, span); ok {
			return native.value, native.err
		}
	}
	obj, ok := receiver.(*instance)
	if !ok {
		return nil, RuntimeError{Message: "unknown method '" + methodName + "'", Span: span}
	}
	methods := instanceMethods(obj)
	for _, method := range methods {
		if method.Name != methodName {
			continue
		}
		if in.runtimeMethodMatches(method, args) {
			return in.callMethod(obj, method, args, local)
		}
	}
	defaultMethods := in.defaultInterfaceMethodsByName(obj, methodName)
	if method, ok := in.findDefaultInterfaceMethod(obj, methodName, args); ok {
		return in.callInterfaceMethod(obj, method, args, local)
	}
	if len(defaultMethods) > 0 {
		return in.callInterfaceMethod(obj, defaultMethods[0], args, local)
	}
	return nil, RuntimeError{Message: "unknown method '" + methodName + "'", Span: span}
}

func (in *Interpreter) evalMember(receiver Value, expr *parser.MemberExpr) (Value, error) {
	switch value := receiver.(type) {
	case moduleRef:
		mod, ok := in.imports[value.name]
		if !ok {
			return nil, RuntimeError{Message: "unknown module '" + value.name + "'", Span: expr.Span}
		}
		if _, ok := mod.functions[expr.Name]; ok {
			return functionRef{module: mod, name: expr.Name}, nil
		}
		if _, ok := mod.classes[expr.Name]; ok {
			return classRef{module: mod, name: expr.Name}, nil
		}
		if _, ok := mod.objects[expr.Name]; ok {
			slot, ok := mod.globals.get(expr.Name)
			if !ok {
				return nil, RuntimeError{Message: "unknown member '" + expr.Name + "'", Span: expr.Span}
			}
			return slot.value, nil
		}
		if mod.publicGlobals[expr.Name] {
			slot, ok := mod.globals.get(expr.Name)
			if !ok {
				return nil, RuntimeError{Message: "unknown member '" + expr.Name + "'", Span: expr.Span}
			}
			return slot.value, nil
		}
		return nil, RuntimeError{Message: "unknown member '" + expr.Name + "'", Span: expr.Span}
	case *instance:
		if field, ok := value.fields[expr.Name]; ok {
			if _, deferred := field.(deferredValue); deferred {
				return nil, RuntimeError{Message: "field '" + expr.Name + "' is deferred and has not been assigned", Span: expr.Span}
			}
			return field, nil
		}
		for _, method := range instanceMethods(value) {
			if method.Name == expr.Name {
				return nil, RuntimeError{Message: "method '" + expr.Name + "' must be called with ()", Span: expr.Span}
			}
		}
		return nil, RuntimeError{Message: "unknown member '" + expr.Name + "'", Span: expr.Span}
	case classRef:
		class, ok := value.module.classes[value.name]
		if !ok {
			return nil, RuntimeError{Message: "unknown class '" + value.name + "'", Span: expr.Span}
		}
		if class.Enum {
			for _, enumCase := range class.Cases {
				if enumCase.Name != expr.Name {
					continue
				}
				if len(enumCase.Fields) == 0 {
					return value.module.constructEnumCase(class, enumCase, nil, value.module.globals, expr.Span)
				}
				return enumCaseRef{module: value.module, enumName: value.name, caseName: expr.Name}, nil
			}
		}
		return nil, RuntimeError{Message: "unknown member '" + expr.Name + "'", Span: expr.Span}
	case *nativeOS:
		switch expr.Name {
		case "stdout":
			return value.stdout, nil
		case "stderr":
			return value.stderr, nil
		}
		if in.nativeHasMethod(receiver, expr.Name) {
			return nil, RuntimeError{Message: "method '" + expr.Name + "' must be called with ()", Span: expr.Span}
		}
		return nil, RuntimeError{Message: "unknown member '" + expr.Name + "'", Span: expr.Span}
	case string, *nativeList, *nativeArray, *nativeOption, *nativeResult, *nativeEither, *nativeSet, *nativeMap, *nativePrinter:
		if in.nativeHasMethod(receiver, expr.Name) {
			return nil, RuntimeError{Message: "method '" + expr.Name + "' must be called with ()", Span: expr.Span}
		}
		return nil, RuntimeError{Message: "unknown member '" + expr.Name + "'", Span: expr.Span}
	case builtinRef:
		if value.name == "Array" && expr.Name == "ofLength" {
			return nil, RuntimeError{Message: "method 'ofLength' must be called with ()", Span: expr.Span}
		}
		return nil, RuntimeError{Message: "unknown member '" + expr.Name + "'", Span: expr.Span}
	case *nativeTuple:
		return nil, RuntimeError{Message: "unknown member '" + expr.Name + "'", Span: expr.Span}
	case *nativeRecord:
		if field, ok := value.fields[expr.Name]; ok {
			return field, nil
		}
		return nil, RuntimeError{Message: "unknown member '" + expr.Name + "'", Span: expr.Span}
	default:
		return nil, RuntimeError{Message: "member access expects class or record instance", Span: expr.Span}
	}
}

func (in *Interpreter) identifierShadowsTypeName(local *env, name string) bool {
	if slot, ok := local.get(name); ok {
		if inst, ok := slot.value.(*instance); ok && inst.class != nil && inst.class.Object && inst.class.Name == name {
			return false
		}
		return slot.value != nil
	}
	if _, ok := in.functions[name]; ok {
		return true
	}
	if _, ok := in.imports[name]; ok {
		return true
	}
	if _, ok := in.classes[name]; ok {
		return true
	}
	if _, ok := in.objects[name]; ok {
		return true
	}
	if _, ok := in.enumCases[name]; ok {
		return true
	}
	return isBuiltinRuntimeValue(name)
}

func isBuiltinRuntimeValue(name string) bool {
	switch name {
	case "List", "Set", "Map", "Array", "OS":
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

func (in *Interpreter) tryEnumCaseMemberFromIdentifier(local *env, typeName, memberName string, span parser.Span) (Value, bool, error) {
	class, ok := in.classes[typeName]
	if !ok || !class.Enum {
		return nil, false, nil
	}
	for _, enumCase := range class.Cases {
		if enumCase.Name != memberName {
			continue
		}
		if len(enumCase.Fields) == 0 {
			value, err := in.constructEnumCase(class, enumCase, nil, local, span)
			return value, true, err
		}
		return enumCaseRef{module: in, enumName: typeName, caseName: memberName}, true, nil
	}
	return nil, false, nil
}

type nativeCallResult struct {
	value Value
	err   error
}

type namedValueArg struct {
	Name  string
	Value Value
	Span  parser.Span
}

func namedArgValues(args []namedValueArg) []Value {
	values := make([]Value, len(args))
	for i, arg := range args {
		values[i] = arg.Value
	}
	return values
}

func hasNamedParserArgs(args []parser.CallArg) bool {
	for _, arg := range args {
		if arg.Name != "" {
			return true
		}
	}
	return false
}

func hasNamedValueArgs(args []namedValueArg) bool {
	for _, arg := range args {
		if arg.Name != "" {
			return true
		}
	}
	return false
}

func reorderNamedValueArgs(params []parser.Parameter, args []namedValueArg, span parser.Span, callable string) ([]Value, error) {
	if len(params) > 0 && params[len(params)-1].Variadic {
		return nil, RuntimeError{Message: "named arguments are not supported for variadic " + callable, Span: span}
	}
	ordered := make([]Value, len(params))
	filled := make([]bool, len(params))
	seenNamed := false
	pos := 0
	for _, arg := range args {
		if arg.Name == "" {
			if seenNamed {
				return nil, RuntimeError{Message: "positional arguments cannot follow named arguments", Span: arg.Span}
			}
			if pos >= len(params) {
				return nil, RuntimeError{Message: fmt.Sprintf("%s expects %d arguments, got %d", callable, len(params), len(args)), Span: span}
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
			return nil, RuntimeError{Message: "unknown named argument '" + arg.Name + "'", Span: arg.Span}
		}
		if filled[paramIndex] {
			return nil, RuntimeError{Message: "argument '" + arg.Name + "' was provided more than once", Span: arg.Span}
		}
		ordered[paramIndex] = arg.Value
		filled[paramIndex] = true
	}
	for i, ok := range filled {
		if !ok {
			return nil, RuntimeError{Message: "missing argument '" + params[i].Name + "' in " + callable, Span: span}
		}
	}
	return ordered, nil
}

func (in *Interpreter) callBuiltin(name string, argExprs []parser.CallArg, args []Value, local *env, span parser.Span) (Value, error) {
	if hasNamedParserArgs(argExprs) {
		return nil, RuntimeError{Message: "named arguments are not supported for builtin constructors", Span: span}
	}
	switch name {
	case "List":
		if args == nil {
			args = make([]Value, len(argExprs))
			for i, arg := range argExprs {
				value, err := in.evalExpr(arg.Value, local)
				if err != nil {
					return nil, err
				}
				args[i] = value
			}
		}
		return &nativeList{items: append([]Value(nil), args...)}, nil
	case "Array":
		if args == nil {
			args = make([]Value, len(argExprs))
			for i, arg := range argExprs {
				value, err := in.evalExpr(arg.Value, local)
				if err != nil {
					return nil, err
				}
				args[i] = value
			}
		}
		return &nativeArray{items: append([]Value(nil), args...)}, nil
	case "Set":
		if args == nil {
			args = make([]Value, len(argExprs))
			for i, arg := range argExprs {
				value, err := in.evalExpr(arg.Value, local)
				if err != nil {
					return nil, err
				}
				args[i] = value
			}
		}
		s := &nativeSet{keys: map[string]Value{}, order: []string{}}
		for _, arg := range args {
			key, err := nativeKey(arg, span, local, in)
			if err != nil {
				return nil, err
			}
			if _, ok := s.keys[key]; !ok {
				s.order = append(s.order, key)
			}
			s.keys[key] = arg
		}
		return s, nil
	case "Some":
		if len(argExprs) != 1 {
			return nil, RuntimeError{Message: fmt.Sprintf("Some constructor expects 1 argument, got %d", len(argExprs)), Span: span}
		}
		if args == nil {
			args = make([]Value, len(argExprs))
			for i, arg := range argExprs {
				value, err := in.evalExpr(arg.Value, local)
				if err != nil {
					return nil, err
				}
				args[i] = value
			}
		}
		return in.constructStdlibOption(args[0], true, local, span)
	case "None":
		if len(argExprs) != 0 {
			return nil, RuntimeError{Message: fmt.Sprintf("None constructor expects 0 arguments, got %d", len(argExprs)), Span: span}
		}
		return in.constructStdlibOption(nil, false, local, span)
	case "Ok":
		if len(argExprs) != 1 {
			return nil, RuntimeError{Message: fmt.Sprintf("Ok constructor expects 1 argument, got %d", len(argExprs)), Span: span}
		}
		if args == nil {
			args = make([]Value, len(argExprs))
			for i, arg := range argExprs {
				value, err := in.evalExpr(arg.Value, local)
				if err != nil {
					return nil, err
				}
				args[i] = value
			}
		}
		return in.constructStdlibResult(args[0], nil, true, local, span)
	case "Err":
		if len(argExprs) != 1 {
			return nil, RuntimeError{Message: fmt.Sprintf("Err constructor expects 1 argument, got %d", len(argExprs)), Span: span}
		}
		if args == nil {
			args = make([]Value, len(argExprs))
			for i, arg := range argExprs {
				value, err := in.evalExpr(arg.Value, local)
				if err != nil {
					return nil, err
				}
				args[i] = value
			}
		}
		return in.constructStdlibResult(nil, args[0], false, local, span)
	case "Left":
		if len(argExprs) != 1 {
			return nil, RuntimeError{Message: fmt.Sprintf("Left constructor expects 1 argument, got %d", len(argExprs)), Span: span}
		}
		if args == nil {
			args = make([]Value, len(argExprs))
			for i, arg := range argExprs {
				value, err := in.evalExpr(arg.Value, local)
				if err != nil {
					return nil, err
				}
				args[i] = value
			}
		}
		return in.constructStdlibEither(args[0], nil, false, local, span)
	case "Right":
		if len(argExprs) != 1 {
			return nil, RuntimeError{Message: fmt.Sprintf("Right constructor expects 1 argument, got %d", len(argExprs)), Span: span}
		}
		if args == nil {
			args = make([]Value, len(argExprs))
			for i, arg := range argExprs {
				value, err := in.evalExpr(arg.Value, local)
				if err != nil {
					return nil, err
				}
				args[i] = value
			}
		}
		return in.constructStdlibEither(nil, args[0], true, local, span)
	case "Map":
		m := &nativeMap{items: map[string]Value{}, keys: map[string]Value{}, order: []string{}}
		for _, argExpr := range argExprs {
			pair, ok := argExpr.Value.(*parser.BinaryExpr)
			if !ok || pair.Operator != ":" {
				return nil, RuntimeError{Message: "Map constructor expects key : value pairs", Span: span}
			}
			keyValue, err := in.evalExpr(pair.Left, local)
			if err != nil {
				return nil, err
			}
			valueValue, err := in.evalExpr(pair.Right, local)
			if err != nil {
				return nil, err
			}
			key, err := nativeKey(keyValue, exprSpan(pair.Left), local, in)
			if err != nil {
				return nil, err
			}
			if _, ok := m.items[key]; !ok {
				m.order = append(m.order, key)
				m.keys[key] = keyValue
			}
			m.items[key] = valueValue
		}
		return m, nil
	default:
		if owner, ok := in.enumCases[name]; ok {
			class, found := in.classes[owner]
			if !found || !class.Enum {
				return nil, RuntimeError{Message: "unknown enum case '" + name + "'", Span: span}
			}
			if args == nil {
				args = make([]Value, len(argExprs))
				for i, arg := range argExprs {
					value, err := in.evalExpr(arg.Value, local)
					if err != nil {
						return nil, err
					}
					args[i] = value
				}
			}
			for _, enumCase := range class.Cases {
				if enumCase.Name == name {
					return in.constructEnumCase(class, enumCase, args, local, span)
				}
			}
			return nil, RuntimeError{Message: "unknown enum case '" + name + "'", Span: span}
		}
		return nil, RuntimeError{Message: "value is not callable", Span: span}
	}
}

func (in *Interpreter) nativeHasMethod(receiver Value, name string) bool {
	_, ok := lookupNativeMethodHandler(receiver, name)
	return ok
}

func (in *Interpreter) nativeMethodParams(receiver Value, name string) ([]parser.Parameter, bool) {
	if typeName, ok := nativeBuiltinTypeName(receiver); ok && typeName == "Array" {
		if _, ok := lookupNativeMethodHandler(receiver, name); ok {
			return nil, true
		}
		return nil, false
	}
	if descriptor, ok := in.nativeMethodDescriptor(receiver, name); ok {
		return append([]parser.Parameter(nil), descriptor.Parameters...), true
	}
	return nil, false
}

func (in *Interpreter) callNativeMethod(receiver Value, name string, args []namedValueArg, local *env, span parser.Span) (nativeCallResult, bool) {
	handler, ok := lookupNativeMethodHandler(receiver, name)
	if !ok {
		return nativeCallResult{}, false
	}
	ordered := namedArgValues(args)
	if hasNamedValueArgs(args) {
		params, ok := in.nativeMethodParams(receiver, name)
		if !ok {
			return nativeCallResult{}, false
		}
		reordered, err := reorderNamedValueArgs(params, args, span, "method '"+name+"'")
		if err != nil {
			return nativeCallResult{err: err}, true
		}
		ordered = reordered
	}
	value, err := handler(in, receiver, ordered, local, span)
	return nativeCallResult{value: value, err: err}, true
}

func builtinRegistry() *predef.Registry {
	registry, err := predef.Load()
	if err != nil {
		panic(err)
	}
	return registry
}

func nativeBuiltinTypeName(value Value) (string, bool) {
	switch value.(type) {
	case string:
		return "Str", true
	case *nativeArray:
		return "Array", true
	case *nativeList:
		return "List", true
	case *nativeListIterator:
		return "Iterator", true
	case *nativeOption:
		return "Option", true
	case *nativeResult:
		return "Result", true
	case *nativeEither:
		return "Either", true
	case *nativeSet:
		return "Set", true
	case *nativeMap:
		return "Map", true
	case *nativePrinter:
		return "Printer", true
	case *nativeOS:
		return "OS", true
	case *instance:
		instanceValue := value.(*instance)
		switch instanceValue.class.Name {
		case "Option":
			return instanceValue.class.Name, true
		}
		return "", false
	default:
		return "", false
	}
}

func lookupBuiltinMethodDescriptor(typeName, methodName string) (predef.MethodDescriptor, bool) {
	return lookupBuiltinMethodDescriptorInRegistry(builtinRegistry(), typeName, methodName, map[string]bool{})
}

func lookupBuiltinMethodDescriptorInRegistry(registry *predef.Registry, typeName, methodName string, seen map[string]bool) (predef.MethodDescriptor, bool) {
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
		if method, ok := lookupBuiltinMethodDescriptorInRegistry(registry, iface.Name, methodName, seen); ok {
			return method, true
		}
	}
	return predef.MethodDescriptor{}, false
}

func builtinTypeImplements(typeName, target string) bool {
	return builtinTypeImplementsInRegistry(builtinRegistry(), typeName, target, map[string]bool{})
}

func builtinTypeImplementsInRegistry(registry *predef.Registry, typeName, target string, seen map[string]bool) bool {
	if typeName == target {
		return true
	}
	if seen[typeName] {
		return false
	}
	seen[typeName] = true
	descriptor, ok := registry.Types[typeName]
	if !ok {
		return false
	}
	for _, iface := range descriptor.ImplementedInterfaces {
		if iface == nil || iface.Name == "" {
			continue
		}
		if iface.Name == target || builtinTypeImplementsInRegistry(registry, iface.Name, target, seen) {
			return true
		}
	}
	return false
}

func (in *Interpreter) nativeMethodDescriptor(receiver Value, name string) (predef.MethodDescriptor, bool) {
	typeName, ok := nativeBuiltinTypeName(receiver)
	if !ok || typeName == "Array" {
		return predef.MethodDescriptor{}, false
	}
	return lookupBuiltinMethodDescriptor(typeName, name)
}

func (in *Interpreter) invokeCallableValue(callee Value, args []Value, local *env, span parser.Span) (Value, error) {
	switch fn := callee.(type) {
	case *closure:
		return in.callClosure(fn, args)
	case builtinRef:
		return in.callBuiltin(fn.name, nil, args, local, span)
	case boundMethodRef:
		return in.invokeBoundMethod(fn.receiver, fn.name, args, local, span)
	case *instance:
		if fn.class != nil && fn.class.Object {
			return nil, RuntimeError{Message: "object '" + fn.class.Name + "' is not callable", Span: span}
		}
		if fn.class != nil {
			return nil, RuntimeError{Message: "value of type '" + fn.class.Name + "' is not callable", Span: span}
		}
		return nil, RuntimeError{Message: "value is not callable", Span: span}
	default:
		return nil, RuntimeError{Message: "value is not callable", Span: span}
	}
}

func nativeKey(value Value, span parser.Span, local *env, in *Interpreter) (string, error) {
	switch v := value.(type) {
	case int64:
		return fmt.Sprintf("i:%d", v), nil
	case float64:
		return fmt.Sprintf("f:%g", v), nil
	case bool:
		if v {
			return "b:true", nil
		}
		return "b:false", nil
	case string:
		return "s:" + v, nil
	case rune:
		return fmt.Sprintf("r:%d", v), nil
	case *instance:
		return fmt.Sprintf("o:%p", v), nil
	default:
		return "", RuntimeError{Message: "unsupported Map/Set key type", Span: span}
	}
}

type moduleRef struct{ name string }
type functionRef struct {
	module *Interpreter
	name   string
}
type boundMethodRef struct {
	receiver Value
	name     string
}
type classRef struct {
	module *Interpreter
	name   string
}

type enumCaseRef struct {
	module   *Interpreter
	enumName string
	caseName string
}

func newEnv(parent *env) *env {
	return &env{parent: parent, values: map[string]slot{}}
}

func cloneEnvShallow(source *env) *env {
	if source == nil {
		return newEnv(nil)
	}
	clone := &env{parent: source.parent, values: map[string]slot{}}
	for name, value := range source.values {
		clone.values[name] = value
	}
	return clone
}

func (e *env) define(name string, value Value, mutable bool) {
	e.values[name] = slot{value: value, mutable: mutable}
}

func (e *env) getLocal(name string) (slot, bool) {
	value, ok := e.values[name]
	return value, ok
}

func (e *env) getUntil(stop *env, name string) (slot, bool) {
	for current := e; current != nil; current = current.parent {
		if value, ok := current.values[name]; ok {
			return value, true
		}
		if current == stop {
			break
		}
	}
	return slot{}, false
}

func (e *env) findThisScope() (*env, *instance, bool) {
	for current := e; current != nil; current = current.parent {
		value, ok := current.values["this"]
		if !ok {
			continue
		}
		obj, ok := value.value.(*instance)
		if !ok {
			return current, nil, false
		}
		return current, obj, true
	}
	return nil, nil, false
}

func (e *env) get(name string) (slot, bool) {
	if value, ok := e.values[name]; ok {
		return value, true
	}
	if e.parent != nil {
		return e.parent.get(name)
	}
	return slot{}, false
}

func (e *env) resolve(name string) (*slotRef, bool) {
	if _, ok := e.values[name]; ok {
		return &slotRef{owner: e, name: name}, true
	}
	if e.parent != nil {
		return e.parent.resolve(name)
	}
	return nil, false
}

type slotRef struct {
	owner *env
	name  string
}

func (r *slotRef) mutable() bool { return r.owner.values[r.name].mutable }
func (r *slotRef) set(value Value) error {
	slot := r.owner.values[r.name]
	slot.value = value
	r.owner.values[r.name] = slot
	return nil
}
func (r *slotRef) value() Value { return r.owner.values[r.name].value }

func acceptsArgCount(params []parser.Parameter, count int) bool {
	if len(params) == 0 {
		return count == 0
	}
	if params[len(params)-1].Variadic {
		return count >= len(params)-1
	}
	return count == len(params)
}

func expectedCallableArgs(params []parser.Parameter) string {
	if len(params) > 0 && params[len(params)-1].Variadic {
		return fmt.Sprintf("at least %d", len(params)-1)
	}
	return fmt.Sprintf("%d", len(params))
}

func acceptsClosureArgCount(fn *closure, count int) bool {
	if fn.variadic {
		return count >= len(fn.params)-1
	}
	if fn.tupleDestructuring && len(fn.params) > 1 && count == 1 {
		return true
	}
	return count == len(fn.params)
}

func expectedClosureArgs(fn *closure) string {
	if fn.variadic {
		return fmt.Sprintf("at least %d", len(fn.params)-1)
	}
	if fn.tupleDestructuring && len(fn.params) > 1 {
		return fmt.Sprintf("1 tuple or %d", len(fn.params))
	}
	return fmt.Sprintf("%d", len(fn.params))
}

func kindOrTuple(kind string) string {
	if kind == "" {
		return "tuple"
	}
	return kind
}

func newNativeOS() *nativeOS {
	return &nativeOS{
		stdout: &nativePrinter{},
		stderr: &nativePrinter{stderr: true},
	}
}

func (in *Interpreter) runtimeMethodMatches(method *parser.MethodDecl, args []Value) bool {
	if !acceptsArgCount(method.Parameters, len(args)) {
		return false
	}
	for i, arg := range args {
		paramIndex := i
		if len(method.Parameters) > 0 && method.Parameters[len(method.Parameters)-1].Variadic && paramIndex >= len(method.Parameters)-1 {
			paramIndex = len(method.Parameters) - 1
		}
		if paramIndex >= len(method.Parameters) {
			return false
		}
		if !in.runtimeValueMatchesType(arg, method.Parameters[paramIndex].Type) {
			return false
		}
	}
	return true
}

func (in *Interpreter) runtimeValueMatchesType(value Value, ref *parser.TypeRef) bool {
	if ref == nil {
		return true
	}
	if ref.ReturnType != nil {
		switch value.(type) {
		case *closure:
			return true
		}
		if instanceValue, ok := value.(*instance); ok {
			for _, method := range instanceValue.class.Methods {
				if method.Name == "apply" {
					return true
				}
			}
		}
		return false
	}
	if len(ref.TupleElements) > 0 {
		tuple, ok := value.(*nativeTuple)
		return ok && len(ref.TupleElements) == len(tuple.items)
	}
	if len(ref.RecordFields) > 0 {
		record, ok := value.(*nativeRecord)
		if !ok {
			return false
		}
		for _, field := range ref.RecordFields {
			fieldValue, exists := record.fields[field.Name]
			if !exists || !in.runtimeValueMatchesType(fieldValue, field.Type) {
				return false
			}
		}
		return true
	}
	switch ref.Name {
	case "", "Unit", "Int", "Float", "Bool", "Str", "Rune", "List", "Iterable", "Iterator", "Set", "Map", "Option", "Result", "Either", "Array", "Printer", "OS":
	default:
		if _, ok := in.classes[ref.Name]; !ok {
			if _, ok := in.interfaces[ref.Name]; !ok {
				return true
			}
		}
	}
	switch ref.Name {
	case "Unit":
		return value == nil
	case "Int":
		_, ok := value.(int64)
		return ok
	case "Float":
		_, ok := value.(float64)
		return ok
	case "Bool":
		_, ok := value.(bool)
		return ok
	case "Str":
		_, ok := value.(string)
		return ok
	case "Rune":
		_, ok := value.(rune)
		return ok
	case "List":
		_, ok := value.(*nativeList)
		return ok
	case "Iterable":
		if _, ok := value.(*nativeArray); ok {
			return true
		}
		if typeName, ok := nativeBuiltinTypeName(value); ok {
			return builtinTypeImplements(typeName, "Iterable")
		}
		return false
	case "Iterator":
		if typeName, ok := nativeBuiltinTypeName(value); ok {
			return builtinTypeImplements(typeName, "Iterator")
		}
		return false
	case "Set":
		if typeName, ok := nativeBuiltinTypeName(value); ok {
			return builtinTypeImplements(typeName, "Set")
		}
		return false
	case "Map":
		if typeName, ok := nativeBuiltinTypeName(value); ok {
			return builtinTypeImplements(typeName, "Map")
		}
		return false
	case "Option":
		if typeName, ok := nativeBuiltinTypeName(value); ok {
			return builtinTypeImplements(typeName, "Option")
		}
		if instanceValue, ok := value.(*instance); ok && instanceValue.class.Name == "Option" {
			return true
		}
		return false
	case "Result":
		if typeName, ok := nativeBuiltinTypeName(value); ok {
			return builtinTypeImplements(typeName, "Result")
		}
		if instanceValue, ok := value.(*instance); ok && instanceValue.class.Name == "Result" {
			return true
		}
		return false
	case "Either":
		if typeName, ok := nativeBuiltinTypeName(value); ok {
			return builtinTypeImplements(typeName, "Either")
		}
		if instanceValue, ok := value.(*instance); ok && instanceValue.class.Name == "Either" {
			return true
		}
		return false
	case "Array":
		_, ok := value.(*nativeArray)
		return ok
	case "OS":
		if typeName, ok := nativeBuiltinTypeName(value); ok {
			return typeName == "OS"
		}
		return false
	case "Printer":
		if typeName, ok := nativeBuiltinTypeName(value); ok {
			return builtinTypeImplements(typeName, "Printer")
		}
		return false
	default:
		if instanceValue, ok := value.(*instance); ok {
			if instanceValue.class.Name == ref.Name {
				return true
			}
			for _, impl := range instanceValue.class.Implements {
				if impl.Name == ref.Name || in.interfaceExtends(impl.Name, ref.Name, map[string]bool{}) {
					return true
				}
			}
		}
		return false
	}
}

func (in *Interpreter) interfaceExtends(name, target string, seen map[string]bool) bool {
	if name == target {
		return true
	}
	if seen[name] {
		return false
	}
	seen[name] = true
	iface, ok := in.interfaces[name]
	if !ok {
		return false
	}
	for _, parent := range iface.Extends {
		if parent.Name == target || in.interfaceExtends(parent.Name, target, seen) {
			return true
		}
	}
	return false
}

func applyBinary(op string, left, right Value, span parser.Span) (Value, error) {
	switch op {
	case "+", "-", "*", "/", "%":
		return applyArithmetic(op, left, right, span)
	case "<", "<=", ">", ">=":
		return compareOrdered(op, left, right, span)
	case "&&":
		lb, err := asBool(left, span)
		if err != nil {
			return nil, err
		}
		rb, err := asBool(right, span)
		if err != nil {
			return nil, err
		}
		if op == "&&" {
			return lb && rb, nil
		}
	case "||":
		lb, err := asBool(left, span)
		if err != nil {
			return nil, err
		}
		rb, err := asBool(right, span)
		if err != nil {
			return nil, err
		}
		return lb || rb, nil
	}
	return nil, RuntimeError{Message: "unsupported operator '" + op + "'", Span: span}
}

func (in *Interpreter) evalOverloadedBinary(op string, left Value, rightExpr parser.Expr, right Value, local *env, span parser.Span) (Value, bool, error) {
	switch op {
	case ":+":
		switch l := left.(type) {
		case *nativeList:
			return &nativeList{items: append(append([]Value(nil), l.items...), right)}, true, nil
		case *nativeSet:
			out := &nativeSet{keys: map[string]Value{}, order: []string{}}
			for _, key := range l.order {
				out.keys[key] = l.keys[key]
				out.order = append(out.order, key)
			}
			key, err := nativeKey(right, span, local, in)
			if err != nil {
				return nil, true, err
			}
			if _, exists := out.keys[key]; !exists {
				out.order = append(out.order, key)
			}
			out.keys[key] = right
			return out, true, nil
		}
	case "++":
		switch l := left.(type) {
		case *nativeList:
			if r, ok := right.(*nativeList); ok {
				out := append(append([]Value(nil), l.items...), r.items...)
				return &nativeList{items: out}, true, nil
			}
		case *nativeSet:
			if r, ok := right.(*nativeSet); ok {
				out := &nativeSet{keys: map[string]Value{}, order: []string{}}
				for _, key := range l.order {
					out.keys[key] = l.keys[key]
					out.order = append(out.order, key)
				}
				for _, key := range r.order {
					if _, exists := out.keys[key]; !exists {
						out.order = append(out.order, key)
					}
					out.keys[key] = r.keys[key]
				}
				return out, true, nil
			}
		case *nativeMap:
			if r, ok := right.(*nativeMap); ok {
				out := &nativeMap{items: map[string]Value{}, keys: map[string]Value{}, order: []string{}}
				for _, key := range l.order {
					out.items[key] = l.items[key]
					out.keys[key] = l.keys[key]
					out.order = append(out.order, key)
				}
				for _, key := range r.order {
					if _, exists := out.items[key]; !exists {
						out.order = append(out.order, key)
					}
					out.items[key] = r.items[key]
					out.keys[key] = r.keys[key]
				}
				return out, true, nil
			}
		}
	case "+", "-", "*", "/", "%", ":-", "--", "|", "&", ">>", "<<", "::":
		if obj, ok := left.(*instance); ok {
			value, err := in.invokeMethod(obj, op, []Value{right}, local, span)
			return value, true, err
		}
	case "~":
		return nil, false, nil
	}
	return nil, false, nil
}

func applyArithmetic(op string, left, right Value, span parser.Span) (Value, error) {
	if ls, ok := left.(string); ok && op == "+" {
		return ls + fmt.Sprint(right), nil
	}
	if rs, ok := right.(string); ok && op == "+" {
		return fmt.Sprint(left) + rs, nil
	}
	if li, ok := left.(int64); ok {
		if ri, ok := right.(int64); ok {
			switch op {
			case "+":
				return li + ri, nil
			case "-":
				return li - ri, nil
			case "*":
				return li * ri, nil
			case "/":
				return li / ri, nil
			case "%":
				return li % ri, nil
			}
		}
	}
	lf, lok := toFloat(left)
	rf, rok := toFloat(right)
	if lok && rok {
		switch op {
		case "+":
			return lf + rf, nil
		case "-":
			return lf - rf, nil
		case "*":
			return lf * rf, nil
		case "/":
			return lf / rf, nil
		case "%":
			return math.Mod(lf, rf), nil
		}
	}
	return nil, RuntimeError{Message: "invalid arithmetic operands", Span: span}
}

func compareOrdered(op string, left, right Value, span parser.Span) (Value, error) {
	if li, ok := left.(int64); ok {
		if ri, ok := right.(int64); ok {
			return compareInts(op, li, ri), nil
		}
	}
	if lf, lok := toFloat(left); lok {
		if rf, rok := toFloat(right); rok {
			return compareFloats(op, lf, rf), nil
		}
	}
	if ls, ok := left.(string); ok {
		if rs, ok := right.(string); ok {
			return compareStrings(op, ls, rs), nil
		}
	}
	if lr, ok := left.(rune); ok {
		if rr, ok := right.(rune); ok {
			return compareInts(op, int64(lr), int64(rr)), nil
		}
	}
	return nil, RuntimeError{Message: "invalid comparison operands", Span: span}
}

func compareInts(op string, left, right int64) bool {
	switch op {
	case "<":
		return left < right
	case "<=":
		return left <= right
	case ">":
		return left > right
	case ">=":
		return left >= right
	default:
		return false
	}
}

func compareFloats(op string, left, right float64) bool {
	switch op {
	case "<":
		return left < right
	case "<=":
		return left <= right
	case ">":
		return left > right
	case ">=":
		return left >= right
	default:
		return false
	}
}

func compareStrings(op, left, right string) bool {
	switch op {
	case "<":
		return left < right
	case "<=":
		return left <= right
	case ">":
		return left > right
	case ">=":
		return left >= right
	default:
		return false
	}
}

func (in *Interpreter) valuesEqual(left, right Value, span parser.Span, local *env) (bool, error) {
	switch l := left.(type) {
	case int64:
		r, ok := right.(int64)
		return ok && l == r, nil
	case float64:
		r, ok := toFloat(right)
		return ok && l == r, nil
	case bool:
		r, ok := right.(bool)
		return ok && l == r, nil
	case string:
		r, ok := right.(string)
		return ok && l == r, nil
	case rune:
		r, ok := right.(rune)
		return ok && l == r, nil
	case *instance:
		r, ok := right.(*instance)
		if !ok || l.class.Name != r.class.Name {
			return false, nil
		}
		if l.class.Enum || r.class.Enum {
			if l.caseName != r.caseName {
				return false, nil
			}
			for name, leftValue := range l.fields {
				equal, err := in.valuesEqual(leftValue, r.fields[name], span, local)
				if err != nil {
					return false, err
				}
				if !equal {
					return false, nil
				}
			}
			return true, nil
		}
		for _, method := range l.class.Methods {
			if method.Name == "equals" && len(method.Parameters) == 1 {
				value, err := in.callMethod(l, method, []Value{r}, local)
				if err != nil {
					return false, err
				}
				if result, ok := value.(bool); ok {
					return result, nil
				}
				return false, RuntimeError{Message: "equals must return Bool", Span: span}
			}
		}
		return false, nil
	default:
		return left == right, nil
	}
}

func toFloat(value Value) (float64, bool) {
	switch n := value.(type) {
	case int64:
		return float64(n), true
	case float64:
		return n, true
	default:
		return 0, false
	}
}

func asBool(value Value, span parser.Span) (bool, error) {
	b, ok := value.(bool)
	if !ok {
		return false, RuntimeError{Message: "expected Bool", Span: span}
	}
	return b, nil
}

func (in *Interpreter) iterableToSlice(value Value, local *env, span parser.Span) ([]Value, bool) {
	if ok, unwrapped, err := in.optionBindingValue(value, local, span); err == nil {
		if !ok {
			return []Value{}, true
		}
		return []Value{unwrapped}, true
	}
	switch items := value.(type) {
	case []Value:
		return items, true
	case *nativeList:
		return items.items, true
	case *nativeArray:
		return items.items, true
	case *nativeTuple:
		if len(items.items) != 2 {
			return nil, false
		}
		start, ok := items.items[0].(int64)
		if !ok {
			return nil, false
		}
		end, ok := items.items[1].(int64)
		if !ok {
			return nil, false
		}
		step := int64(1)
		if start > end {
			step = -1
		}
		var out []Value
		for current := start; ; current += step {
			if step > 0 {
				if current >= end {
					break
				}
			} else if current <= end {
				break
			}
			out = append(out, current)
		}
		return out, true
	case *nativeSet:
		out := make([]Value, 0, len(items.order))
		for _, key := range items.order {
			out = append(out, items.keys[key])
		}
		return out, true
	}
	iterator, err := in.invokeMethod(value, "iterator", nil, local, span)
	if err != nil {
		return nil, false
	}
	var out []Value
	for {
		hasNextValue, err := in.invokeMethod(iterator, "hasNext", nil, local, span)
		if err != nil {
			return nil, false
		}
		hasNext, ok := hasNextValue.(bool)
		if !ok || !hasNext {
			break
		}
		nextValue, err := in.invokeMethod(iterator, "next", nil, local, span)
		if err != nil {
			return nil, false
		}
		out = append(out, nextValue)
	}
	return out, true
}

func (in *Interpreter) invokeMethod(receiver Value, name string, args []Value, local *env, span parser.Span) (Value, error) {
	named := make([]namedValueArg, len(args))
	for i, arg := range args {
		named[i] = namedValueArg{Value: arg, Span: span}
	}
	if native, ok := in.callNativeMethod(receiver, name, named, local, span); ok {
		return native.value, native.err
	}
	obj, ok := receiver.(*instance)
	if !ok {
		return nil, RuntimeError{Message: "member call requires class instance", Span: span}
	}
	for _, method := range instanceMethods(obj) {
		if method.Name != name {
			continue
		}
		if in.runtimeMethodMatches(method, args) {
			return in.callMethod(obj, method, args, local)
		}
	}
	if method, ok := in.findDefaultInterfaceMethod(obj, name, args); ok {
		return in.callInterfaceMethod(obj, method, args, local)
	}
	return nil, RuntimeError{Message: "unknown method '" + name + "'", Span: span}
}

func (in *Interpreter) findDefaultInterfaceMethod(receiver *instance, name string, args []Value) (parser.InterfaceMethod, bool) {
	candidates := in.defaultInterfaceMethodsByName(receiver, name)
	for _, method := range candidates {
		if in.runtimeInterfaceMethodMatches(method, args) {
			return method, true
		}
	}
	return parser.InterfaceMethod{}, false
}

func (in *Interpreter) defaultInterfaceMethodsByName(receiver *instance, name string) []parser.InterfaceMethod {
	var (
		found []parser.InterfaceMethod
	)
	seen := map[string]bool{}
	for _, impl := range receiver.class.Implements {
		found = append(found, in.findDefaultInterfaceMethodsInRef(impl, name, seen)...)
	}
	return found
}

func (in *Interpreter) findDefaultInterfaceMethodsInRef(ref *parser.TypeRef, name string, seen map[string]bool) []parser.InterfaceMethod {
	if ref == nil || seen[ref.Name] {
		return nil
	}
	seen[ref.Name] = true
	iface, ok := in.interfaces[ref.Name]
	if !ok {
		return nil
	}
	var found []parser.InterfaceMethod
	for _, method := range iface.Methods {
		if method.Name == name && method.Body != nil {
			found = append(found, method)
		}
	}
	for _, parent := range iface.Extends {
		found = append(found, in.findDefaultInterfaceMethodsInRef(parent, name, seen)...)
	}
	return found
}

func (in *Interpreter) runtimeInterfaceMethodMatches(method parser.InterfaceMethod, args []Value) bool {
	if !acceptsArgCount(method.Parameters, len(args)) {
		return false
	}
	for i, arg := range args {
		paramIndex := i
		if len(method.Parameters) > 0 && method.Parameters[len(method.Parameters)-1].Variadic && paramIndex >= len(method.Parameters)-1 {
			paramIndex = len(method.Parameters) - 1
		}
		if paramIndex >= len(method.Parameters) {
			return false
		}
		if !in.runtimeValueMatchesType(arg, method.Parameters[paramIndex].Type) {
			return false
		}
	}
	return true
}

func (in *Interpreter) constructStdlibOption(value Value, set bool, local *env, span parser.Span) (Value, error) {
	class, ok := in.classes["Option"]
	if !ok {
		if set {
			return &nativeOption{value: value, set: true}, nil
		}
		return &nativeOption{set: false}, nil
	}
	if class.Enum {
		caseName := "None"
		args := []Value{}
		if set {
			caseName = "Some"
			args = []Value{value}
		}
		for _, enumCase := range class.Cases {
			if enumCase.Name == caseName {
				return in.constructEnumCase(class, enumCase, args, local, span)
			}
		}
		return nil, RuntimeError{Message: "stdlib Option enum is missing case '" + caseName + "'", Span: span}
	}
	if set {
		return in.construct(class, []Value{value}, local)
	}
	return in.construct(class, nil, local)
}

func (in *Interpreter) constructStdlibResult(value Value, errValue Value, ok bool, local *env, span parser.Span) (Value, error) {
	class, found := in.classes["Result"]
	if !found {
		return &nativeResult{value: value, err: errValue, ok: ok}, nil
	}
	if class.Enum {
		caseName := "Err"
		args := []Value{errValue}
		if ok {
			caseName = "Ok"
			args = []Value{value}
		}
		for _, enumCase := range class.Cases {
			if enumCase.Name == caseName {
				return in.constructEnumCase(class, enumCase, args, local, span)
			}
		}
		return nil, RuntimeError{Message: "stdlib Result enum is missing case '" + caseName + "'", Span: span}
	}
	obj, err := in.constructBuiltinInstance(class, local)
	if err != nil {
		return nil, err
	}
	obj.fields["ok"] = ok
	if ok {
		obj.fields["value"] = value
		obj.fields["error"] = zeroValue(fieldTypeByName(class, "error"))
	} else {
		obj.fields["value"] = zeroValue(fieldTypeByName(class, "value"))
		obj.fields["error"] = errValue
	}
	return obj, nil
}

func (in *Interpreter) constructStdlibEither(left Value, right Value, rightSet bool, local *env, span parser.Span) (Value, error) {
	class, found := in.classes["Either"]
	if !found {
		return &nativeEither{left: left, right: right, rightSet: rightSet}, nil
	}
	if class.Enum {
		caseName := "Left"
		args := []Value{left}
		if rightSet {
			caseName = "Right"
			args = []Value{right}
		}
		for _, enumCase := range class.Cases {
			if enumCase.Name == caseName {
				return in.constructEnumCase(class, enumCase, args, local, span)
			}
		}
		return nil, RuntimeError{Message: "stdlib Either enum is missing case '" + caseName + "'", Span: span}
	}
	obj, err := in.constructBuiltinInstance(class, local)
	if err != nil {
		return nil, err
	}
	obj.fields["rightSet"] = rightSet
	if rightSet {
		obj.fields["left"] = zeroValue(fieldTypeByName(class, "left"))
		obj.fields["right"] = right
	} else {
		obj.fields["left"] = left
		obj.fields["right"] = zeroValue(fieldTypeByName(class, "right"))
	}
	return obj, nil
}

func (in *Interpreter) constructBuiltinInstance(class *parser.ClassDecl, parent *env) (*instance, error) {
	obj := &instance{class: class, fields: map[string]Value{}}
	fieldEnv := newEnv(parent)
	fieldEnv.define("this", obj, false)
	for _, field := range class.Fields {
		switch {
		case field.Initializer != nil:
			value, err := in.evalExpr(field.Initializer, fieldEnv)
			if err != nil {
				return nil, err
			}
			obj.fields[field.Name] = value
		case field.Deferred:
			obj.fields[field.Name] = deferredValue{}
		default:
			obj.fields[field.Name] = zeroValue(field.Type)
		}
	}
	return obj, nil
}

func fieldTypeByName(class *parser.ClassDecl, name string) *parser.TypeRef {
	for i := range class.Fields {
		if class.Fields[i].Name == name {
			return class.Fields[i].Type
		}
	}
	return nil
}

func (in *Interpreter) instanceImplements(value *instance, target string) bool {
	if value.class.Name == target {
		return true
	}
	for _, impl := range value.class.Implements {
		if impl.Name == target || in.interfaceExtends(impl.Name, target, map[string]bool{}) {
			return true
		}
	}
	return false
}

func (in *Interpreter) bindingValues(bindings []parser.Binding, values []parser.Expr, local *env, span parser.Span) ([]Value, error) {
	if len(bindings) == 0 || len(values) == 0 {
		return nil, nil
	}
	if len(bindings) == len(values) {
		out := make([]Value, len(values))
		for i, expr := range values {
			if expr == nil {
				out[i] = deferredValue{}
				continue
			}
			value, err := in.evalExprWithTypeRef(expr, bindingTypeRef(bindings, i), local)
			if err != nil {
				return nil, err
			}
			value = in.coerceValueForBinding(bindingTypeRef(bindings, i), value)
			out[i] = value
		}
		return out, nil
	}
	if len(values) == 1 {
		if values[0] == nil {
			return nil, RuntimeError{Message: "cannot destructure deferred value", Span: span}
		}
		value, err := in.evalExpr(values[0], local)
		if err != nil {
			return nil, err
		}
		items, kind, ok := destructurableValues(value)
		if !ok {
			return nil, RuntimeError{Message: fmt.Sprintf("binding expects %d values, got 1", len(bindings)), Span: span}
		}
		if len(items) != len(bindings) {
			return nil, RuntimeError{Message: fmt.Sprintf("binding expects %d %s values, got %d", len(bindings), kind, len(items)), Span: span}
		}
		return append([]Value(nil), items...), nil
	}
	return nil, RuntimeError{Message: fmt.Sprintf("binding expects %d values, got %d", len(bindings), len(values)), Span: span}
}

func (in *Interpreter) evalExprWithTypeRef(expr parser.Expr, expected *parser.TypeRef, local *env) (Value, error) {
	if record, ok := expr.(*parser.AnonymousRecordExpr); ok && len(record.Values) > 0 && expected != nil && len(expected.RecordFields) > 0 {
		if len(record.Values) != len(expected.RecordFields) {
			return nil, RuntimeError{Message: fmt.Sprintf("record(...) expects %d values, got %d", len(expected.RecordFields), len(record.Values)), Span: record.Span}
		}
		fields := make(map[string]Value, len(expected.RecordFields))
		order := make([]string, len(expected.RecordFields))
		for i, field := range expected.RecordFields {
			value, err := in.evalExprWithTypeRef(record.Values[i], field.Type, local)
			if err != nil {
				return nil, err
			}
			fields[field.Name] = value
			order[i] = field.Name
		}
		return &nativeRecord{fields: fields, order: order}, nil
	}
	if lambda, ok := expr.(*parser.LambdaExpr); ok {
		params := make([]string, len(lambda.Parameters))
		for i, param := range lambda.Parameters {
			params[i] = param.Name
		}
		return &closure{
			params:             params,
			tupleDestructuring: len(lambda.Parameters) > 1,
			body:               lambda.Body,
			blockBody:          lambda.BlockBody,
			env:                local,
		}, nil
	}
	return in.evalExpr(parser.WrapContextualFunctionExpr(expected, expr), local)
}

func (in *Interpreter) evalArgsWithParams(params []parser.Parameter, argExprs []parser.CallArg, local *env, span parser.Span, callable string) ([]Value, error) {
	orderedExprs := make([]parser.Expr, len(argExprs))
	if hasNamedParserArgs(argExprs) {
		reordered, err := reorderNamedParserArgs(params, argExprs, span, callable)
		if err != nil {
			return nil, err
		}
		orderedExprs = reordered
	} else {
		for i, arg := range argExprs {
			orderedExprs[i] = arg.Value
		}
	}
	values := make([]Value, len(orderedExprs))
	for i, expr := range orderedExprs {
		var expected *parser.TypeRef
		if i < len(params) {
			expected = params[i].Type
		}
		value, err := in.evalExprWithTypeRef(expr, expected, local)
		if err != nil {
			return nil, err
		}
		values[i] = value
	}
	return values, nil
}

func reorderNamedParserArgs(params []parser.Parameter, args []parser.CallArg, span parser.Span, callable string) ([]parser.Expr, error) {
	ordered := make([]parser.Expr, len(params))
	filled := make([]bool, len(params))
	positional := 0
	seenNamed := false
	for _, arg := range args {
		if arg.Name == "" {
			if seenNamed {
				return nil, RuntimeError{Message: "positional arguments cannot follow named arguments in " + callable, Span: arg.Span}
			}
			if positional >= len(params) {
				return nil, RuntimeError{Message: "too many arguments in " + callable, Span: arg.Span}
			}
			ordered[positional] = arg.Value
			filled[positional] = true
			positional++
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
			return nil, RuntimeError{Message: "unknown named argument '" + arg.Name + "'", Span: arg.Span}
		}
		if filled[paramIndex] {
			return nil, RuntimeError{Message: "argument '" + arg.Name + "' was provided more than once", Span: arg.Span}
		}
		ordered[paramIndex] = arg.Value
		filled[paramIndex] = true
	}
	for i, ok := range filled {
		if !ok {
			return nil, RuntimeError{Message: "missing argument '" + params[i].Name + "' in " + callable, Span: span}
		}
	}
	return ordered, nil
}

func bindingTypeRef(bindings []parser.Binding, index int) *parser.TypeRef {
	if index < 0 || index >= len(bindings) {
		return nil
	}
	return bindings[index].Type
}

func (in *Interpreter) coerceValueForBinding(ref *parser.TypeRef, value Value) Value {
	return in.coerceValueForTypeRef(ref, value)
}

func isUnitTypeRef(ref *parser.TypeRef) bool {
	return ref != nil && ref.Name == "Unit" && len(ref.Arguments) == 0 && len(ref.TupleElements) == 0 && ref.ReturnType == nil
}

func (in *Interpreter) coerceValueForTypeRef(ref *parser.TypeRef, value Value) Value {
	if ref == nil {
		return value
	}
	if isUnitTypeRef(ref) {
		return nil
	}
	return value
}

func (in *Interpreter) recordMatchesVisibleClassShape(class *parser.ClassDecl, record *nativeRecord) bool {
	required, optional, ok := in.visibleClassShape(class)
	if !ok {
		return false
	}
	if len(record.fields) < len(required) || len(record.fields) > len(required)+len(optional) {
		return false
	}
	optionalSet := map[string]*parser.TypeRef{}
	for _, field := range optional {
		optionalSet[field.Name] = field.Type
	}
	for _, field := range required {
		value, ok := record.fields[field.Name]
		if !ok || !in.runtimeValueMatchesType(value, field.Type) {
			return false
		}
	}
	for name, value := range record.fields {
		requiredField := false
		for _, field := range required {
			if field.Name == name {
				requiredField = true
				break
			}
		}
		if requiredField {
			continue
		}
		fieldType, ok := optionalSet[name]
		if !ok || !in.runtimeValueMatchesType(value, fieldType) {
			return false
		}
	}
	return true
}

func (in *Interpreter) visibleClassShape(class *parser.ClassDecl) (required []parser.FieldDecl, optional []parser.FieldDecl, ok bool) {
	for _, field := range class.Fields {
		if field.Private {
			if field.Initializer == nil {
				return nil, nil, false
			}
			continue
		}
		if field.Initializer != nil {
			optional = append(optional, field)
			continue
		}
		required = append(required, field)
	}
	return required, optional, true
}

func (in *Interpreter) assignmentValues(targetCount int, values []parser.Expr, local *env, span parser.Span) ([]Value, error) {
	if targetCount == len(values) {
		out := make([]Value, len(values))
		for i, expr := range values {
			value, err := in.evalExpr(expr, local)
			if err != nil {
				return nil, err
			}
			out[i] = value
		}
		return out, nil
	}
	if len(values) == 1 {
		value, err := in.evalExpr(values[0], local)
		if err != nil {
			return nil, err
		}
		items, kind, ok := destructurableValues(value)
		if !ok {
			return nil, RuntimeError{Message: fmt.Sprintf("assignment expects %d values, got 1", targetCount), Span: span}
		}
		if len(items) != targetCount {
			return nil, RuntimeError{Message: fmt.Sprintf("assignment expects %d %s values, got %d", targetCount, kind, len(items)), Span: span}
		}
		return append([]Value(nil), items...), nil
	}
	return nil, RuntimeError{Message: fmt.Sprintf("assignment expects %d values, got %d", targetCount, len(values)), Span: span}
}

func destructurableValues(value Value) ([]Value, string, bool) {
	if tuple, ok := value.(*nativeTuple); ok {
		return tuple.items, "tuple", true
	}
	instanceValue, ok := value.(*instance)
	if !ok || instanceValue.class.Enum || instanceValue.class.Object {
		return nil, "", false
	}
	for _, field := range instanceValue.class.Fields {
		if field.Private {
			return nil, "", false
		}
	}
	items := make([]Value, len(instanceValue.class.Fields))
	for i, field := range instanceValue.class.Fields {
		fieldValue, ok := instanceValue.fields[field.Name]
		if !ok {
			return nil, "", false
		}
		items[i] = fieldValue
	}
	return items, "destructured", true
}

func indexedItems(value Value) ([]Value, bool) {
	switch items := value.(type) {
	case []Value:
		return items, true
	case *nativeList:
		return items.items, true
	case *nativeArray:
		return items.items, true
	default:
		return nil, false
	}
}

func zeroValue(ref *parser.TypeRef) Value {
	if ref == nil {
		return nil
	}
	switch ref.Name {
	case "Int", "Int64":
		return int64(0)
	case "Float", "Float64":
		return float64(0)
	case "Bool":
		return false
	case "Str":
		return ""
	case "Rune":
		return rune(0)
	case "Unit":
		return nil
	case "List":
		return &nativeList{items: []Value{}}
	case "Set":
		return &nativeSet{keys: map[string]Value{}, order: []string{}}
	case "Map":
		return &nativeMap{items: map[string]Value{}, keys: map[string]Value{}, order: []string{}}
	case "Array":
		return &nativeArray{items: []Value{}}
	case "Tuple":
		items := make([]Value, len(ref.TupleElements))
		for i, elem := range ref.TupleElements {
			items[i] = zeroValue(elem)
		}
		return &nativeTuple{items: items}
	case "Record":
		fields := make(map[string]Value, len(ref.RecordFields))
		order := make([]string, 0, len(ref.RecordFields))
		for _, field := range ref.RecordFields {
			order = append(order, field.Name)
			fields[field.Name] = zeroValue(field.Type)
		}
		return &nativeRecord{fields: fields, order: order}
	case "Option":
		return &nativeOption{set: false}
	case "OS":
		return newNativeOS()
	case "Printer":
		return &nativePrinter{}
	default:
		return nil
	}
}

func stmtSpan(stmt parser.Statement) parser.Span {
	switch s := stmt.(type) {
	case *parser.ValStmt:
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
	case *parser.TupleLiteral:
		return e.Span
	case *parser.CallExpr:
		return e.Span
	case *parser.MemberExpr:
		return e.Span
	case *parser.IndexExpr:
		return e.Span
	case *parser.RecordUpdateExpr:
		return e.Span
	case *parser.LambdaExpr:
		return e.Span
	case *parser.BinaryExpr:
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
