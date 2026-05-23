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

The Rust implementation now has three real frontend pieces:

- a lexer that tokenizes Lume source and reports lexical diagnostics
- a recursive-descent parser that builds a source-shaped AST
- a semantic resolver that performs early name binding and structure checks

The type checker, lowering, and interpreter modules are still scaffolded so we
have a clean place to keep building without reworking the crate layout later.

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

The `check` command resolves the requested file and its imports, installs
ambient stdlib names from `stdlib/*.lum`, and reports early semantic diagnostics
such as:

- duplicate top-level declarations
- duplicate or shadowing local bindings
- undefined value names
- undefined type names
- generic arity mismatches
- invalid `break` outside a loop
- unknown imported module members

## Near-Term Direction

The intended next steps are:

1. define a typed semantic layer that does not depend on source-shaped nodes
2. lower that typed tree into a compact interpreter-oriented IR
3. execute the lowered IR in Rust rather than interpreting the source AST
4. port more of the stdlib/runtime behavior onto the Rust path

That keeps the Rust implementation aligned with the direction we discussed:
optimize one implementation path instead of maintaining multiple backends.
