# Rust Implementation

This folder is the start of a Rust implementation of `Lume`.

The goal is not to mirror the Go code file-for-file. The goal is to keep the
same broad pipeline while moving toward a runtime that is easier to optimize:

```txt
source
-> lexer
-> parser
-> semantic checks
-> type checking
-> lowered IR
-> interpreter
```

## Current Scope

The Rust implementation now has six real building blocks:

- a lexer that tokenizes Lume source and reports lexical diagnostics
- a recursive-descent parser that builds a source-shaped AST
- a semantic resolver that performs early name binding and structure checks
- a first-pass type checker for values, calls, constructors, imports, and
  common control-flow forms
- a real interpreter-oriented IR with program, type, global, function, local,
  block, statement, terminator, operand, and rvalue structures
- a first lowering pass that maps declarations plus real control-flow bodies
  into that IR

The interpreter is still scaffolded. Lowering is now real, but intentionally
partial: functions, methods, globals, `if`, `while`, `match`, `for`,
`for ... yield`, `unwrap`, assignments, calls, and core expressions lower into
CFG blocks, while richer forms like lambdas, local functions, anonymous
interfaces, and record updates still report targeted lowering diagnostics.

## Layout

```txt
rust/
  Cargo.toml
  README.md
  crates/
    lume/
      Cargo.toml
      src/
        main.rs
        lib.rs
        source.rs
        diagnostic.rs
        lexer.rs
        ast.rs
        parser.rs
        resolver.rs
        typecheck.rs
        ir.rs
        lower.rs
        interpreter.rs
```

## Running It

From the repository root:

```bash
cargo run --manifest-path rust/Cargo.toml -p lume -- tokens hello.lum
cargo run --manifest-path rust/Cargo.toml -p lume -- parse examples/random_code/bumper.lum
cargo run --manifest-path rust/Cargo.toml -p lume -- check examples/import_forms.lum
```

The `tokens` command prints the token stream with spans.

The `parse` command lexes, parses, and pretty-prints the AST for the requested
file. Right now it covers:

- packages and imports
- top-level functions, types, impl blocks, and top-level bindings
- class/record/object/interface/enum declarations
- fields, methods, enum cases, and impl methods
- blocks, bindings, assignments, `if`, `while`, `for`, `return`, and `break`
- calls, member access, indexing, lists, tuples, lambdas, and `if` expressions

The `check` command resolves and type-checks the requested file and its imports,
installs ambient stdlib names from `stdlib/*.lum`, and reports diagnostics such
as:

- duplicate top-level declarations
- duplicate or shadowing local bindings
- undefined value names
- undefined type names
- generic arity mismatches
- argument and return type mismatches
- invalid assignment and binding types
- incorrect constructor arity and named arguments
- invalid `break` outside a loop
- unknown imported module members

The library also has a `lower_program(...)` entry point that produces the new
IR and already lowers the main statement/control-flow surface used by the Rust
tests. It still reports explicit diagnostics for features that are not
implemented yet on the Rust path.

## Near-Term Direction

The intended next steps are:

1. strengthen the type checker so it covers more of the full language surface
2. widen lowering coverage for lambdas, local functions, and record updates
3. execute the lowered IR in Rust rather than interpreting the source AST
4. port more of the stdlib/runtime behavior onto the Rust path

That keeps the Rust implementation aligned with the direction we discussed:
optimize one implementation path instead of maintaining multiple backends.
