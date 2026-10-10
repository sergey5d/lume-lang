# Rust Implementation

This folder contains the active Rust implementation of `Lume`. It owns the
shared frontend, formatter, IR interpreter, and readable Java generator:

```txt
source
-> lexer
-> parser
-> resolver
-> type checker
-> body-level Core desugaring
-> readable Java
   or lowered IR -> interpreter
```

## Current Scope

The Rust implementation now has these real building blocks:

- a lexer that tokenizes Lume source and reports lexical diagnostics
- a recursive-descent parser that builds a source-shaped AST
- a semantic resolver that performs early name binding and structure checks
- a type checker for values, calls, constructors, use declarations, and
  common control-flow forms
- a real interpreter-oriented IR with program, type, global, function, local,
  block, statement, terminator, operand, and rvalue structures
- a Core desugaring pass that normalizes callable bodies before lowering
- a lowering pass that maps declarations plus Core bodies into IR
- a real IR interpreter that executes lowered multi-module programs with
  globals, user-defined types/methods, `match`, `for`, `for ... yield`,
  `try`, extraction operators, closures, shape updates, use declarations, and the stdlib/runtime helpers
  needed by the checked-in examples
- a source-shaped Java generator with shared backend capability diagnostics
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
        diagnostic_render.rs
        lexer.rs
        formatter.rs
        ast.rs
        parser/
        resolver.rs
        core.rs
        desugar.rs
        typecheck.rs
        ir.rs
        lower.rs
        backend/
        java_backend/
        runtime/
        interpreter.rs
```

## Running It

From the repository root:

```bash
cargo run --manifest-path rust/Cargo.toml -p lume -- tokens examples/os.lum
cargo run --manifest-path rust/Cargo.toml -p lume -- parse examples/random_code/bumper.lum
cargo run --manifest-path rust/Cargo.toml -p lume -- fmt examples/random_code/bumper.lum
cargo run --manifest-path rust/Cargo.toml -p lume -- check examples/import_forms.lum
cargo run --manifest-path rust/Cargo.toml -p lume -- run examples/range.lum
cargo run --manifest-path rust/Cargo.toml -p lume -- test examples/unit_tests.lum
cargo run --manifest-path rust/Cargo.toml -p lume -- gen examples/range.lum --out build/generated/lume
```

`gen` produces readable, source-shaped Java. Unsupported method bodies are
reported as compilation diagnostics instead of producing partial Java output.

The `tokens` command prints the token stream with spans.

The `fmt` command validates and formats one source file in place. It preserves
comments and multiline string contents and does not overwrite invalid source.

The `parse` command lexes, parses, and pretty-prints the AST for the requested
file. Right now it covers:

- modules and use declarations
- top-level functions, types, extension blocks, and top-level bindings
- class/object/interface declarations and declared unions
- fields, methods, constructors, and union alternatives in declaration bodies
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

The library also has a `lower_program(...)` entry point that produces the
IR used by the interpreter and the Rust tests.

The `run` command executes the lowered IR for the current Rust implementation.
It supports:

- top-level globals and entry functions (`main` by default, then `run`)
- user-defined classes/objects and declared unions with methods
- `if`, `while`, `match`, `for`, `for ... yield`, `defer`,
  `return`, `break`, and `continue`
- `try`, `??`, and postfix `!` over `Option`, `Result`, and `Either`
- builtin constructors and helpers like `Range`, `Vector`, `Array`, `Some`, `None`,
  `Ok`, `Err`, `Left`, `Right`, and `OS.println`
- imported-module execution through the resolver/runtime merge path
- string interpolation, multiline strings, and `%`-style `printf`

## Near-Term Direction

The intended next steps are:

1. remove the remaining latent `unsupported` branches in lowering/runtime
2. widen unexercised stdlib/runtime behavior beyond the current sample set
3. tighten diagnostics and runtime parity across the checked-in examples
4. keep the interpreter and readable-Java paths aligned behind one frontend
