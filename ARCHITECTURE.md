# Architecture

This repository now has a single implementation path: the Rust toolchain under `rust/`.

The high-level flow is:

```txt
source
-> lexer
-> parser
-> resolver
-> type checker
-> lowered IR
-> interpreter
```

## Main Pieces

- `rust/crates/lume/src/lexer.rs`
  Tokenizes `.lum` source and reports lexical diagnostics.
- `rust/crates/lume/src/parser/`
  Builds the source-shaped AST.
- `rust/crates/lume/src/resolver.rs`
  Performs name binding and early structural checks.
- `rust/crates/lume/src/typecheck.rs`
  Resolves types, calls, constructors, and module imports.
- `rust/crates/lume/src/ir.rs`
  Defines the interpreter-oriented IR.
- `rust/crates/lume/src/lower.rs`
  Lowers AST programs into IR.
- `rust/crates/lume/src/interpreter.rs`
  Executes lowered IR and wires in runtime behavior.

## Repository Assets Around It

- `examples/` contains runnable and failure fixtures used by parity tests.
- `stdlib/` contains ambient standard library source loaded during checking and execution.
- `syntax.md` and `features.md` describe the language surface independently from the implementation.

For command usage and current implementation scope, see [rust/README.md](/Users/sergeyd/Projects/a-lang/rust/README.md).
