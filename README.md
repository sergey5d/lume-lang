# Lume

`Lume` is a small experimental programming language implemented in Go.

The repository currently contains:

- a lexer and parser
- semantic name resolution
- a type checker
- a typed AST builder
- a lowering pass to a backend-friendly IR
- a tree-walking interpreter
- an early Go code generator

The interpreter is the most practical execution path right now.

There is also a new Rust implementation scaffold under `rust/README.md`. It starts with a real lexer and parser entrypoint, and is intended to grow toward a lowered-IR interpreter.

## Current CLI

Run a program by calling its `main` function:

```bash
go run . example.lum
```

Run a different entry function:

```bash
go run . example.lum run myEntry
```

Print the parsed AST instead of running:

```bash
go run . example.lum ast
```

Before running, the CLI always:

1. parses the source
2. runs semantic resolution
3. runs type checking
4. aborts if diagnostics are reported

## Sample Sweeps

Run the checked example suite through the Rust interpreter:

```bash
./run_samples.sh rust
```

Run the existing Go example suite:

```bash
./run_samples.sh go
```

Run both:

```bash
./run_samples.sh both
```

## Example

```txt
class Counter {
	hidden var count Int
}

impl Counter {
	def init(count Int) {
		this.count = count
	}

	def inc() Int {
		this.count += 1
		return this.count
	}
}

seed Int = 1

def main(input Int) Int {
	counter Counter = Counter(input)
	if input > 0 {
		return counter.inc()
	}
	return seed
}
```

## Language Snapshot

The syntax is still evolving, but the current codebase supports a meaningful subset:

- immutable bindings with `=`
- mutable bindings with `var ... =`
- compound assignment like `+=`
- deferred initialization with `?`
- functions with `def`
- classes and interfaces
- `with` for interface implementation, including multiple interfaces on one class or record
- `hidden` members
- generics on classes and interfaces
- lambdas, including block-bodied lambdas
- function types like `Int -> Int`
- tuples, including named tuple fields
- indexing syntax `arr[i]`
- `while true` for infinite loops
- `for` loops, including yield-style loops
- typed equality rules via `Eq[T]`

Some representative forms:

```txt
count Int = 1
var total Int = 0

addOne Int -> Int = x -> x + 1

values = Array(1, 2, 3)
values[1] := values[0] + 4

pair (value Int, size Int) = (1, 2)
renamed (left Int, right Int) = pair
total Int = pair.value + renamed.left
```

## Tuple Rules

The current tuple model is structural by element type and order:

- unnamed tuple types use `(Int, Str)`
- named tuple types use `(value Int, label Str)`
- tuple literals use `(1, "ok")`
- tuples can be destructured into multiple bindings

Tuple names are preserved on the current typed view of a value:

```txt
a (value Int, size Int) = (1, 2)
b (Int, Int) = a
c = a
d (left Int, right Int) = a
```

With these rules:

- `a.value` is valid
- `b.value` is invalid
- `c.value` is valid because inferred bindings keep tuple names from the initializer
- `d.left` is valid because an explicit destination type can rename tuple fields
- assignment compatibility ignores tuple field names and only checks element count and element types
- duplicate names inside one named tuple type are invalid

## Repository Layout

- `parser/`
  Token definitions, lexer, AST, and parser.

- `semantic/`
  Name resolution and scope validation. This pass reports diagnostics but does not transform the tree.

- `typecheck/`
  Type analysis and semantic validation. This also records expression types used by later passes.

- `typed/`
  Builds a typed semantic tree from the parser AST plus type-checker results.

- `lower/`
  Converts the typed AST into a simpler IR used by backends.

- `interpreter/`
  Tree-walking interpreter for the language. This is currently the most complete way to execute programs.

- `codegen/golang/`
  Early Go backend that emits Go source from lowered IR.

- `main.go`
  CLI entrypoint.

## Pipeline

The code currently follows this flow:

```txt
source
-> lexer
-> parser AST
-> semantic resolver
-> type checker
-> typed AST
-> lowered IR
-> interpreter or code generator
```

In practice:

- the interpreter still executes directly from the parser AST
- the Go backend consumes lowered IR

## Current Architecture In More Detail

### 1. Parser

`parser.Parse` converts source text into `parser.Program`.

This tree is close to source syntax. It still contains high-level constructs such as:

- lambdas
- class declarations
- yield-style loops
- method calls
- list literals

### 2. Semantic Resolver

`semantic.Analyze` checks:

- undefined names
- duplicate declarations
- scope visibility
- assignment-to-immutable binding errors

This pass is analysis-only.

### 3. Type Checker

`typecheck.Analyze` checks:

- declared vs inferred binding types
- operator compatibility
- call argument counts and types
- interface conformance
- class equality via `Eq`
- constructor initialization rules
- hidden access rules
- array indexing rules

This pass is also analysis-only. It does not rewrite the AST.

### 4. Typed AST

`typed.Build` creates a new semantic tree that carries:

- resolved types
- symbol IDs
- resolved call/member targets
- binding modes and init modes

This is the first major transformation layer.

### 5. Lowering

`lower.ProgramFromTyped` converts the typed AST into a smaller IR.

The lowered IR makes backend work simpler by normalizing:

- declarations
- assignments
- calls
- method calls
- indexing
- lambdas
- function-value invocation
- yield loops

For example, yield loops are lowered into:

- a synthesized result list
- nested `ForEach` loops
- `append`-style accumulation

### 6. Interpreter

The interpreter is currently the best-supported runtime.

It supports:

- functions
- classes and methods
- lambdas
- equality via `equals`
- indexing
- for loops
- yield loops

### 7. Go Codegen

The Go backend is now capable of generating valid Go for the currently lowered subset, but it should still be considered early-stage.

It already handles:

- functions
- classes as structs
- constructors
- methods
- lambdas
- invoke expressions
- indexing
- yield-lowered loops

## Tests

Run the full test suite:

```bash
go test ./...
```

## Current Priorities

The project is now better positioned to move in either of two directions:

- interpreter-first language stabilization
- broader backend/codegen work

At the moment, the interpreter path is the most useful one for validating language semantics quickly.
