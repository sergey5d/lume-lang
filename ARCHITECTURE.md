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
        diagnostic_render.rs  terminal diagnostic rendering
        lexer.rs              tokenization
        formatter.rs          source formatter
        ast.rs                source-shaped syntax tree
        parser/               recursive-descent parser
        resolver.rs           module loading + name resolution
        typecheck.rs          semantic/type checking
        core.rs               first desugared body-level IR
        desugar.rs            AST -> Core desugaring
        ir.rs                 execution-oriented IR
        lower.rs              AST/Core -> IR lowering
        backend/              shared backend metadata and capabilities
        java_backend/         readable Java generation
        stdlib/               native contracts, capabilities, and implementations
        runtime/              dense per-program runtime type metadata
        interpreter.rs        IR interpreter
  stdlib/                     source counterparts and Java runtime generation
  examples/                   runnable and failing language fixtures
  vscode-extension/           editor syntax/config support
  SYNTAX.md                   language reference
  features.md                 feature notes and status
```

## End-to-End Flow

The checked frontend feeds two execution paths:

```txt
source file(s)
-> lexer
-> parser
-> AST
-> resolver
-> type checker
   -> readable Java generator
   or
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
- obtains ambient stdlib contracts from the Rust-authored descriptor registry
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
  `RuntimeProgram`, `RuntimeType`, `RuntimeField`, `RuntimeMethod`, and declared-
  union variant metadata.
- `stdlib/declarations/` supplies complete contracts for native types,
  constructors, methods, getters, and companions.
- `stdlib/native/` registers native implementation functions. Runtime method
  names, parameter names/types, return types, getter kinds, and generic
  information are derived from the contracts rather than placeholder signatures.

One useful mental model is:

- IR owns user program structure and method bodies.
- Runtime metadata owns fast execution-time lookup layout.

Runtime methods can point either to lowered IR functions or to builtin Rust
functions. Both occupy dense method slots; stdlib descriptors are reusable
contracts, while runtime IDs and field layouts belong to one program.

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

### 10. Java backend

`java_backend/` emits readable, source-shaped Java from the resolved and checked
program. Shared backend descriptors and capability checks live in `backend/`.
Java generation deliberately does not consume the interpreter IR: the two
backends share the frontend but preserve representations suited to their output.

## Execution Model

The CLI in `main.rs` exposes seven user-facing entry points:

- `tokens` for raw lexing output
- `parse` for AST inspection
- `fmt` for in-place source formatting
- `check` for module-aware semantic/type validation
- `run` for full checked execution
- `test` for running Lume spec files
- `gen` for readable Java generation

There is currently no bytecode VM or native machine-code backend. `run` uses the
IR interpreter, while `gen` emits Java for JVM compilation.

## Standard Library and Builtins

Native library contracts are authored in `rust/crates/lume/src/stdlib/`.
Each type has an explicit declaration containing its constructors, members,
generic parameters and bounds, named parameters, defaults, and visibility.
These declarations enter normal resolution and call checking, just like user
declarations and the Java stub adapter's declarations. Ordinary factories and
methods do not need member-specific typechecker signatures.

`stdlib/operators.rs` attaches explicit intrinsic capability entries to types.
The compiler still understands arithmetic, comparison, extraction, indexing,
and iteration syntax; the registered type contract determines which intrinsic
operations are supported. Closed native representations are trusted core
bindings, not interfaces that arbitrary user values can implement.

The interpreter builds its runtime tables from these contracts and registered
native function bindings. Ordinary member calls use those tables rather than
central switches on type and method names. Operation implementations may still
inspect runtime values, just as lowered user methods inspect their arguments.

Ambient contracts and the native JSON/regex module contracts do not require
external `.lum` files. JSON and regex retain their explicit import scopes.
`LUME_STDLIB` and ancestor-directory lookup remain useful for explicit source
imports; source counterparts in `stdlib/*.lum` also support navigation and
Java runtime generation, but are not the interpreter's signature authority.

The implementation plan and migration boundaries are recorded in
[stdlib-descriptors-plan.md](docs/stdlib-descriptors-plan.md).

## Examples, Tests, and Docs

- `examples/` contains both successful programs and `examples/failures/`
  fixtures used by parser, typecheck, and interpreter parity tests.
- `run_samples.sh` sweeps checked examples from the repo root.
- Many Rust modules keep unit tests next to the implementation, and
  `parser/tests.rs` holds parser-focused coverage.
- `SYNTAX.md` is the main language reference.
- `features.md` tracks general open language work, while focused proposal files
  such as `async_proposal.md` retain designs that need more room.

## Current Design Boundary

The most important architectural boundary in the codebase today is:

- AST is the source language model.
- Core is the first cleanup/desugaring layer for callable bodies.
- IR is the execution model.
- Runtime metadata is the interpreter's dense lookup model.
- The Java backend consumes checked source structure rather than interpreter IR.

That separation is what keeps the current implementation understandable while
still leaving room to grow toward a more explicit Core-first or VM-like future.
