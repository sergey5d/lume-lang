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

The Rust implementation now has seven real building blocks:

- a lexer that tokenizes Lume source and reports lexical diagnostics
- a recursive-descent parser that builds a source-shaped AST
- a semantic resolver that performs early name binding and structure checks
- a first-pass type checker for values, calls, constructors, imports, and
  common control-flow forms
- a real interpreter-oriented IR with program, type, global, function, local,
  block, statement, terminator, operand, and rvalue structures
- a first lowering pass that maps declarations plus real control-flow bodies
  into that IR
- a real IR interpreter that executes lowered multi-module programs with
  globals, user-defined types/methods, `match`, `for`, `for ... yield`,
  `unwrap`, closures, record updates, imports, and the stdlib/runtime helpers
  needed by the checked-in examples
- a repo-wide Rust parity test that runs non-skipped `examples/*.lum` files and
  compares output against their `# EXPECT:` headers

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
cargo run --manifest-path rust/Cargo.toml -p lume -- run examples/range.lum
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
IR used by the interpreter and the Rust tests.

The `run` command executes the lowered IR for the current Rust implementation.
It supports:

- top-level globals and entry functions (`main` by default, then `run`)
- user-defined classes/records/objects/enums with methods
- `if`, `while`, `match`, `for`, `for ... yield`, `return`, and `break`
- `unwrap` over `Option`, `Result`, and `Either`
- builtin constructors and helpers like `Range`, `List`, `Some`, `None`,
  `Ok`, `Err`, `Left`, `Right`, and `OS.println`
- imported-module execution through the resolver/runtime merge path
- string interpolation, multiline strings, and `%`-style `printf`

## Near-Term Direction

The intended next steps are:

1. tighten output parity further by comparing more behavior against the Go path
2. remove the remaining latent `unsupported` branches in lowering/runtime
3. widen unexercised stdlib/runtime behavior beyond the current sample set
4. decide whether Rust should replace only the interpreter path or also the
   backend/codegen tooling

That keeps the Rust implementation aligned with the direction we discussed:
optimize one implementation path instead of maintaining multiple backends.
