# Architecture

This repository currently has one active implementation path: the Rust
toolchain under `rust/`. Everything else in the repo supports that path:
language docs, stdlib source, runnable examples, tests, and editor tooling.

## Repository Shape

```txt
a-lang/
  rust/
    Cargo.toml                Rust workspace
    crates/lume/
      src/
        main.rs               CLI entry point
        lib.rs                library exports
        source.rs             source files and spans
        diagnostic.rs         diagnostics
        lexer.rs              tokenization
        ast.rs                source-shaped syntax tree
        parser/               recursive-descent parser
        resolver.rs           module loading + name resolution
        typecheck.rs          semantic/type checking
        core.rs               first desugared body-level IR
        desugar.rs            AST -> Core desugaring
        ir.rs                 execution-oriented IR
        lower.rs              AST/Core -> IR lowering
        runtime/              runtime type metadata + builtin methods
        interpreter.rs        IR interpreter
  stdlib/                     language-visible standard library source
  examples/                   runnable and failing language fixtures
  vscode-extension/           editor syntax/config support
  syntax.md                   language reference
  features.md                 feature notes and status
```

## End-to-End Flow

The current runtime pipeline is:

```txt
source file(s)
-> lexer
-> parser
-> AST
-> resolver
-> type checker
-> body-level desugaring to Core
-> lowering to IR
-> runtime metadata assembly
-> IR interpreter
```

Two important details:

- Resolver and type checker still operate on the source-shaped AST.
- Core is currently a body-level lowering boundary, not a full replacement for
  the top-level AST.

## Major Layers

### 1. Frontend infrastructure

- `source.rs` defines `SourceFile`, positions, and spans.
- `diagnostic.rs` defines errors/warnings shared by every phase.
- `lexer.rs` converts source text into tokens.

This layer is intentionally small and shared by every later pass.

### 2. Parser and source AST

- `parser/mod.rs` owns parser state and top-level program parsing.
- `parser/items.rs`, `stmt.rs`, `expr.rs`, `types.rs`, `pattern.rs`,
  `strings.rs`, and `support.rs` split the grammar by concern.
- `ast.rs` defines the source-facing tree exactly enough for syntax-oriented
  passes and diagnostics.

The AST is the language surface model. It still remembers source-only shapes
like grouped expressions, parser call style flags, and lambda body variants.

### 3. Resolver

`resolver.rs` handles source loading and early semantic structure:

- loads modules and `use` declarations into a `ModuleGraph`
- reads ambient stdlib declarations from `stdlib/`
- checks symbol visibility and module/member access
- performs early name-binding and structural diagnostics

`resolve_path(...)` is the main module-aware resolver entry point.

### 4. Type checker

`typecheck.rs` builds a semantic world from the loaded modules and checks:

- type references and generic arity
- bindings and assignments
- calls, methods, constructors, and function/method named arguments
- control-flow forms such as `if`, `match`, `for`, and `let`
- `use` declaration and visibility rules

`check_program(...)` works on an in-memory AST program.
`check_path(...)` is the full module-aware path that the CLI and runtime use.

### 5. Core and desugaring

`core.rs` is the first desugared representation. Right now it focuses on
callable bodies, blocks, statements, and expressions.

`desugar.rs` converts AST bodies into Core and currently normalizes a few
surface-only details:

- grouped expressions are removed
- call syntax becomes semantic `CallStyle::{Paren, Brace}`
- lambda bodies become one normalized expression form

This gives lowering a cleaner input without forcing the whole compiler to
switch away from AST all at once.

### 6. IR

`ir.rs` defines the execution-oriented intermediate representation used by the
interpreter. It contains stable ids and explicit program structure for:

- programs
- types and fields
- globals
- functions, locals, and captures
- basic blocks
- statements and terminators
- operands, places, constants, and rvalues

IR is the first representation that looks like execution rather than syntax.

### 7. Lowering

`lower.rs` bridges the checked frontend world into IR.

The current split is:

- top-level declarations still start from AST
- function and method bodies are desugared into Core first
- lowering emits explicit control flow, temporaries, calls, matches, loops,
  constructors, and closure captures as IR

So the practical shape today is:

```txt
AST program
-> Core callable bodies
-> IR program
```

### 8. Runtime metadata

`runtime/` builds the dense runtime view used by the interpreter hot path.

- `runtime/types.rs` converts `ir::Program` type information into
  `RuntimeProgram`, `RuntimeType`, `RuntimeField`, `RuntimeMethod`, and enum
  case metadata.
- `runtime/builtins/` provides builtin runtime types and host-implemented
  methods for currently hardcoded builtins such as `Str`, `Option`, `Result`,
  `Either`, `List`, `Set`, and `Map`.

One useful mental model is:

- IR owns user program structure and method bodies.
- Runtime metadata owns fast execution-time lookup layout.

Runtime methods can point either to lowered IR functions or to builtin Rust
functions.

### 9. Interpreter

`interpreter.rs` executes the lowered IR.

At startup it:

- checks modules with `check_path(...)`
- loads and merges runtime modules for execution
- lowers the merged program to IR
- builds `RuntimeProgram::from_ir(...)`
- runs the chosen entry function

The interpreter executes IR, not AST and not Core. Core only exists to make
lowering cleaner.

## Execution Model

The CLI in `main.rs` exposes four user-facing entry points:

- `tokens` for raw lexing output
- `parse` for AST inspection
- `check` for module-aware semantic/type validation
- `run` for full checked execution

There is currently no separate bytecode VM or native backend. The interpreter
is the execution engine.

## Standard Library and Builtins

There are two different "standard library" layers:

- `stdlib/*.lum` contains language-visible source declarations that participate
  in parsing, resolving, checking, and imported execution.
- `runtime/builtins/*.rs` contains host-side implementations for builtin
  runtime behavior the interpreter must execute directly.

That split keeps the interpreter slimmer: user code still lowers to IR, while
builtin behavior is registered as runtime methods instead of being hardcoded
throughout the interpreter.

## Examples, Tests, and Docs

- `examples/` contains both successful programs and `examples/failures/`
  fixtures used by parser, typecheck, and interpreter parity tests.
- `run_samples.sh` sweeps checked examples from the repo root.
- Many Rust modules keep unit tests next to the implementation, and
  `parser/tests.rs` holds parser-focused coverage.
- `syntax.md` is the main language reference.
- `features.md` and the proposal markdown files at the repo root track design
  status and open language questions.

## Current Design Boundary

The most important architectural boundary in the codebase today is:

- AST is the source language model.
- Core is the first cleanup/desugaring layer for callable bodies.
- IR is the execution model.
- Runtime metadata is the interpreter's dense lookup model.

That separation is what keeps the current implementation understandable while
still leaving room to grow toward a more explicit Core-first or VM-like future.
