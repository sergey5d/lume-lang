# Rust Implementation

This folder is the start of a Rust implementation of `a-lang`.

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

The first concrete piece implemented here is the lexer plus a tiny CLI that can
print tokens for a source file.

The parser, type checker, lowering, and interpreter modules are scaffolded so
we have a clean place to keep building without reworking the crate layout later.

## Layout

```txt
rust/
  Cargo.toml
  README.md
  crates/
    alang/
      Cargo.toml
      src/
        main.rs
        lib.rs
        source.rs
        diagnostic.rs
        lexer.rs
        ast.rs
        parser.rs
        typecheck.rs
        ir.rs
        lower.rs
        interpreter.rs
```

## Running It

From the repository root:

```bash
cargo run --manifest-path rust/Cargo.toml -p alang -- tokens hello.al
```

That command lexes the file and prints the token stream with spans.

## Near-Term Direction

The intended next steps are:

1. grow the parser from token streaming into a real `Program` builder
2. define a typed semantic layer that does not depend on source-shaped nodes
3. lower that typed tree into a compact interpreter-oriented IR
4. execute the lowered IR in Rust rather than interpreting the source AST

That keeps the Rust implementation aligned with the direction we discussed:
optimize one implementation path instead of maintaining multiple backends.

