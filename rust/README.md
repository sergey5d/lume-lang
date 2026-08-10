# Rust Implementation

This folder contains the active Rust implementation of `Lume`.

The goal is not to mirror the Go code file-for-file. The goal is to keep the
same broad pipeline while moving toward a runtime that is easier to optimize:

```txt
source
-> lexer
-> parser
-> resolver
-> type checker
-> body-level Core desugaring
-> lowered IR
-> runtime metadata
-> interpreter
```

## Current Scope

The Rust implementation now has these real building blocks:

- a lexer that tokenizes Lume source and reports lexical diagnostics
- a recursive-descent parser that builds a source-shaped AST
- a semantic resolver that performs early name binding and structure checks
- a first-pass type checker for values, calls, constructors, use declarations, and
  common control-flow forms
- a real interpreter-oriented IR with program, type, global, function, local,
  block, statement, terminator, operand, and rvalue structures
- a Core desugaring pass that normalizes callable bodies before lowering
- a lowering pass that maps declarations plus Core bodies into IR
- a real IR interpreter that executes lowered multi-module programs with
  globals, user-defined types/methods, `match`, `for`, `for ... yield`,
  `try`, `expect`, closures, shape updates, use declarations, and the stdlib/runtime helpers
  needed by the checked-in examples
- a repo-wide Rust parity test that runs non-skipped `examples/*.lum` files and
  validates `# EXPECT:`, `# FAIL:`, and `# FAIL_REGEX:` headers

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
        parser/
        resolver.rs
        core.rs
        desugar.rs
        typecheck.rs
        ir.rs
        lower.rs
        runtime/
        interpreter.rs
```

## Running It

From the repository root:

```bash
cargo run --manifest-path rust/Cargo.toml -p lume -- tokens examples/os.lum
cargo run --manifest-path rust/Cargo.toml -p lume -- parse examples/random_code/bumper.lum
cargo run --manifest-path rust/Cargo.toml -p lume -- check examples/import_forms.lum
cargo run --manifest-path rust/Cargo.toml -p lume -- run examples/range.lum
```

The `tokens` command prints the token stream with spans.

The `parse` command lexes, parses, and pretty-prints the AST for the requested
file. Right now it covers:

- modules and use declarations
- top-level functions, types, extension blocks, and top-level bindings
- class/object/interface/enum declarations
- fields, methods, constructors, and enum cases in declaration bodies
- blocks, bindings, assignments, `if`, `while`, `for`, `defer`, `return`,
  `break`, and `continue`
- calls, member access, indexing, vectors, arrays, tuples, lambdas, and `if` expressions

The `check` command resolves and type-checks the requested file and its `use` dependencies,
installs ambient stdlib names from `stdlib/*.lum`, and reports diagnostics such
as:

- duplicate top-level declarations
- duplicate or shadowing local bindings
- undefined value names
- undefined type names
- generic arity mismatches
- argument and return type mismatches
- invalid assignment and binding types
- incorrect constructor arity and function/method named arguments
- invalid `break` outside a loop
- unknown used module members

The library also has a `lower_program(...)` entry point that produces the new
IR used by the interpreter and the Rust tests.

The `run` command executes the lowered IR for the current Rust implementation.
It supports:

- top-level globals and entry functions (`main` by default, then `run`)
- user-defined classes/objects/enums with methods
- `if`, `while`, `match`, `partial`, `for`, `for ... yield`, `defer`,
  `return`, `break`, and `continue`
- `try` propagation and `expect` assertions over `Option`, `Result`, and `Either`
- builtin constructors and helpers like `Range`, `Vector`, `Array`, `Some`, `None`,
  `Ok`, `Err`, `Left`, `Right`, and `OS.println`
- imported-module execution through the resolver/runtime merge path
- string interpolation, multiline strings, and `%`-style `printf`

## Near-Term Direction

The intended next steps are:

1. remove the remaining latent `unsupported` branches in lowering/runtime
2. widen unexercised stdlib/runtime behavior beyond the current sample set
3. tighten diagnostics and runtime parity across the checked-in examples
4. keep simplifying the implementation around one maintained runtime path
