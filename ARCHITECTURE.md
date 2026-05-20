# Architecture

This document explains how the codebase is organized and how the major parts interact.

The short version is:

```txt
source
-> parser
-> semantic resolver
-> type checker
-> typed AST builder
-> lowering
-> interpreter or code generator
```

The details below describe what each package owns and what it expects from the others.

## High-Level Flow

### 1. Source Text

The entrypoint starts with a `.lum` source file.

`main.go` reads the file and then drives the front-end pipeline:

1. `parser.Parse`
2. `semantic.Analyze`
3. `typecheck.Analyze`

If semantic or type diagnostics exist, execution stops before interpretation.

If the CLI is in `run` mode, the checked `parser.Program` is then passed to the interpreter.
If the CLI is in `ast` mode, the parser AST is printed as JSON.

## Package Responsibilities

## `parser`

Files in `parser/` define:

- tokens
- lexer
- parser
- parser AST nodes
- source spans

### What it produces

`parser.Parse` returns `*parser.Program`.

This is the first full tree for the program.
It is still source-shaped and preserves syntax-level distinctions such as:

- `CallExpr`
- `MemberExpr`
- `IndexExpr`
- `LambdaExpr`
- `ForStmt`
- class declarations
- interface declarations

### What it does not do

The parser does not know:

- whether names are defined
- what types expressions have
- which overload a call resolves to
- whether a class actually implements an interface

That work belongs to later passes.

## `semantic`

Files in `semantic/` implement name resolution and scope validation.

### Input

- `*parser.Program`

### Output

- `[]Diagnostic`

### What it checks

- undefined names
- duplicate bindings
- duplicate declarations
- visibility of names through nested scopes
- assignment to immutable bindings
- basic validity of type references

### What it does not do

The resolver is an analysis pass only.
It does not:

- mutate the parser AST
- build a typed tree
- attach durable symbol references back onto nodes

It uses temporary scope/type-scope state internally while traversing, then returns diagnostics.

## `typecheck`

Files in `typecheck/` implement type analysis and deeper semantic validation.

### Input

- `*parser.Program`

### Output

- `typecheck.Result`

`typecheck.Result` currently contains:

- `Diagnostics`
- `ExprTypes`

### What it checks

- declared vs inferred binding types
- operator compatibility
- argument counts and argument types
- constructor calls
- method calls
- field assignment rules
- `private` access rules
- interface conformance
- equality rules via `Eq`
- lambda typing
- indexing rules for `Array[T]`

### Important relationship with later stages

The checker is still an analysis pass, but it produces reusable semantic data.

The most important current artifact is:

- `ExprTypes map[parser.Expr]*Type`

That map is used by the typed AST builder as part of the semantic source of truth.

## `typed`

Files in `typed/` create a semantic tree on top of the parser AST and checker output.

### Input

- `*parser.Program`
- `typecheck.Result`

### Output

- `*typed.Program`

### Why this layer exists

The parser AST is still too syntax-oriented for backend work.

The typed AST resolves many ambiguities by turning syntax into semantically meaningful nodes.

Examples:

- constructor calls become `ConstructorCallExpr`
- method calls become `MethodCallExpr`
- plain function calls become `FunctionCallExpr`
- field reads become `FieldExpr`
- function-value invocation becomes `InvokeExpr`

### What else it adds

The typed tree also carries:

- resolved types on expressions
- symbol IDs
- binding/init modes
- resolved method/function/class targets where available

This is the first transformation layer in the codebase.

## `lower`

Files in `lower/` convert the typed AST into a smaller backend-oriented IR.

### Input

- `*typed.Program`

### Output

- `*lower.Program`

### Why this layer exists

Backends usually do not want to reason directly about source-shaped or even typed-source-shaped trees.

The lowering step simplifies execution/codegen by producing a smaller set of constructs.

Examples:

- declarations become `VarDecl`
- assignments become `Assign`
- loops become `ForEach` or `Loop`
- yield loops become explicit accumulation into a list
- lambdas become lowered `Lambda`
- function-value invocation becomes lowered `Invoke`

### Current important lowering choices

- multi-binding `for` loops are lowered into nested `ForEach` loops
- yield loops are lowered into:
  - a synthesized result variable
  - nested iteration
  - builtin `append` accumulation

### Relationship to backends

Both code generation and any future IR-based interpreter can use `lower.Program`.

## `interpreter`

Files in `interpreter/` implement a tree-walking runtime.

### Current input

- `*parser.Program`

### Current output

- runtime values from calling an entry function, which defaults to `main` in the CLI

### Why it still uses parser AST

The interpreter existed before the typed/lowered backend pipeline was fully established.
It currently runs directly from parser nodes rather than from typed AST or lowered IR.

That means there are effectively two execution models in the repo:

- parser-AST interpreter
- typed/lowered backend path

### What it currently supports

- functions
- classes and methods
- lambdas
- indexing
- equality via `equals`
- `while`
- ordinary `for`
- yield-style `for`

### Important interaction with the front-end

The interpreter itself does not run semantic/type checking.
The CLI does that first and only constructs the interpreter if diagnostics are clear.

So runtime execution assumes the program has already passed front-end validation.

## `codegen/golang`

Files in `codegen/golang/` generate Go source from lowered IR.

### Input

- `*lower.Program`

### Output

- formatted Go source

### Why it depends on lowering instead of parser AST

By the time codegen runs, it already wants:

- resolved call kinds
- normalized loop structure
- explicit lambda/invoke nodes
- backend-friendly statement forms

The lowered IR provides that.

### Current status

This backend is early, but it already supports the currently lowered subset:

- globals
- functions
- classes as Go structs
- constructors
- methods
- lambdas
- invoke expressions
- indexing
- yield-lowered loops

## `main.go`

`main.go` is the CLI coordinator.

It is intentionally thin and mostly just wires packages together.

### Current modes

- `run` (executes the `main` entry function by default)
- `ast`

### Current execution path

`run` mode:

1. parse source
2. semantic analyze
3. type check
4. build interpreter
5. call entry function (default `main`)

`ast` mode:

1. parse source
2. semantic analyze
3. type check
4. print parser AST as JSON

## Data Shapes Across Layers

It helps to think of the project as having three different tree/IR shapes:

### 1. Parser AST

Owned by `parser`.

Purpose:
- preserve source structure

Examples:
- `parser.CallExpr`
- `parser.MemberExpr`
- `parser.LambdaExpr`

### 2. Typed AST

Owned by `typed`.

Purpose:
- preserve source structure while attaching semantic meaning

Examples:
- `typed.FunctionCallExpr`
- `typed.ConstructorCallExpr`
- `typed.MethodCallExpr`
- `typed.InvokeExpr`

### 3. Lowered IR

Owned by `lower`.

Purpose:
- give backends a smaller, more uniform representation

Examples:
- `lower.VarDecl`
- `lower.Assign`
- `lower.ForEach`
- `lower.Lambda`
- `lower.Invoke`

## Current Cross-Package Dependencies

The important dependency direction is:

```txt
parser
-> semantic
-> typecheck
-> typed
-> lower
-> codegen/golang
```

The interpreter is the main exception because it still depends directly on `parser`.

So the practical current picture is:

```txt
parser -> semantic -> typecheck
parser -> interpreter
parser + typecheck -> typed -> lower -> codegen/golang
```

## Why Some Pieces Look Redundant

At first glance it may seem like the project has both:

- an interpreter
- a typed AST
- a lowered IR
- a code generator

That is because the codebase is in a transition from “front-end plus direct interpreter” toward a cleaner staged architecture.

Right now:

- the interpreter is the best runtime
- the typed/lowered/codegen path is becoming the long-term backend architecture

Those two paths overlap, but they serve different stages of the project.

## Good Mental Model For Future Work

When changing the language, the safest order is usually:

1. update parser AST/syntax
2. update semantic resolver
3. update type checker
4. update typed AST builder
5. update lowering
6. update interpreter and/or codegen
7. add tests at the relevant layers

That keeps the architecture aligned and avoids having one layer silently drift away from the others.

## Current Biggest Architectural Gap

The main architectural mismatch still present is:

- interpreter executes parser AST
- backend path executes lowered IR

That is acceptable for now, but if the project continues interpreter-first for a while, a future cleanup may be:

- interpret from typed AST
- or interpret from lowered IR

That would reduce duplicated semantic behavior between the interpreter and the backend pipeline.
