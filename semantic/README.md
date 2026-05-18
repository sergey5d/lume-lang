# Semantic Resolver

[`resolver.go`](semantic/resolver.go) is the early semantic pass that runs after parsing and before typechecking.

Its job is to answer basic structure questions like:

- is this name defined?
- is this type name defined?
- does this declaration shadow something it should not?
- is this `break` / `return` / constructor-only use in a valid place?
- does this type reference have the right number of type arguments?

It does **not** compute expression types or overload resolution. That belongs to the typechecker.

## What It Produces

Today the resolver is primarily a validator pass.

Its direct output is:

- a slice of semantic diagnostics

It does **not** currently produce a reusable exported artifact such as:

- a typed AST
- a persisted symbol table reused by later phases
- resolved expression metadata consumed by typechecking or interpretation

The resolver does maintain rich internal state while it runs, but that state is
ephemeral and local to the pass. Later stages rebuild the information they need
instead of consuming a stored resolver result.

## What It Tracks

The resolver maintains a few parallel namespaces:

- value scopes: local variables and bindings
- type scopes: type parameters and locally visible type names
- top-level declarations: functions, classes, objects, interfaces
- imported names: module aliases, direct symbol imports, imported types
- ambient predef names: stdlib types and values exposed through `predef`

Those are represented by fields on `Resolver` such as:

- `scopes`
- `typeScopes`
- `globals`
- `functions`
- `classes`
- `objects`
- `interfaces`
- `imports`
- `importedGlobals`
- `importedClasses`
- `importedObjects`
- `importedInterfaces`
- `classTypes`
- `ifaceTypes`
- `ambientValues`

## Two Entry Points

- `Analyze(program *parser.Program)`
  Use this for a single parsed file/program.

- `AnalyzeModule(mod *module.LoadedModule)`
  Use this for module-aware analysis with imports and prelude loading already resolved by the module loader.

Both paths build a fresh `Resolver`, install ambient stdlib names from `predef`, then walk the program.

## Ambient Stdlib

`installAmbientPredef()` pulls declarations from [`predef.Load()`](predef/registry.go) and makes selected stdlib names visible to the resolver without requiring explicit source imports.

This is how declarations like `Option`, `Either`, `Result`, tuples, `OS`, `Iterable`, and similar ambient stdlib names can participate in semantic resolution even when they are not merged into each user file as source prelude.

The resolver only installs declarations marked for interpreter/ambient use by predef directives.

## Imports

There are two import-related helpers:

- `moduleImportInfo(...)`
  Builds the namespace visible through module aliases like `foo.Bar`.

- `installDirectImports(...)`
  Installs direct symbol imports into the local resolver state, including direct object-member imports.

This separation keeps module-member access and direct imported-name access distinct.

## Type Resolution

`resolveTypeRef(...)` is the core type-name validation routine.

It handles:

- function type refs
- tuple type refs
- anonymous record type refs
- generic arity checking
- local type parameters
- visible class/interface names
- builtin primitive types

If a type is missing, this is the pass that emits `undefined_type`.

## Name Resolution

When visiting expressions, the resolver checks whether identifiers refer to:

- local bindings
- globals
- functions
- classes / objects / interfaces
- imports
- ambient stdlib names

If not, it emits `undefined_name`.

This pass does not decide *which overload* a call means or *what type* an expression has. It only establishes whether a reference is semantically valid enough to move on.

## Typical Diagnostics From This Pass

Examples of diagnostics the resolver is responsible for:

- `undefined_name`
- `undefined_type`
- `invalid_type_arity`
- shadowing diagnostics
- duplicate record field/type-parameter style structural issues
- control-flow placement issues like invalid `break` / `return` contexts

## What To Change Here vs In Typecheck

Change the resolver when the problem is about:

- visibility
- declaration existence
- scope structure
- import exposure
- generic arity
- contextual validity of syntax constructs

Change the typechecker when the problem is about:

- concrete expression/result types
- assignability
- overload resolution
- method signatures
- pattern typing
- inference

## Current Mental Model

The resolver is the "is this program structurally meaningful?" pass.

The typechecker is the "does this meaningful program type-check?" pass.
