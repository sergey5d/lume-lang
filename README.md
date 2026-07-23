# Lume

`Lume` is an experimental programming language implemented in Rust. The current
repo focuses on the compiler/interpreter pipeline, language design notes,
checked examples, a small stdlib, and VS Code syntax support.

The language aims to stay direct and readable. It is designed around a few
separate ideas rather than one large object model trying to do everything.

For the language reference, start with [syntax.md](syntax.md). For implementation
details, start with [ARCHITECTURE.md](ARCHITECTURE.md) and
[rust/README.md](rust/README.md).

## Language Model

Lume separates the main concepts deliberately:

- `class` is the nominal runtime type: it owns identity, fields, visibility, and
  class construction.
- `shape` is structural data: visible read-only fields that can be matched,
  passed, and converted by field compatibility.
- `enum` models tagged alternatives, while `single` models one singleton value
  with optional fields and methods.
- `interface` describes behavior; types opt in explicitly instead of gaining
  behavior by accidental structural matches.

Construction is its own idea:

- `new { ... }` declares the input shape accepted by a class constructor.
- `Type { field: value }` fills that constructor shape by field name.
- `Type(value)` fills the same constructor shape by declaration order.
- If a class has no explicit `new`, the compiler synthesizes field construction
  from visible fields.

Control flow is expression-friendly but still explicit:

- `match` is exhaustive; `partial match` returns `Option`.
- `let` and `if let` are pattern-oriented binding forms;
  `assert(...)` handles boolean assertions.
- `try` and lifted access (`.->`) handle `Option`, `Result`, and `Either` flow
  without turning every method call into special syntax.

## Quick Start

Run the CLI from the repository root:

```bash
cargo run --manifest-path rust/Cargo.toml -p lume -- parse examples/constructors.lum
cargo run --manifest-path rust/Cargo.toml -p lume -- check examples/random_code/asset_prices.lum
cargo run --manifest-path rust/Cargo.toml -p lume -- run examples/range.lum
```

Run tests:

```bash
cargo test --manifest-path rust/Cargo.toml -p lume
./run_samples.sh
```

## Small Taste

```txt
shape Point {
    x Int
    y Int
}

class User {
    name Str
    home Point
}

impl User {
    new {
        name Str
        home Point
    } {
        this.name = name
        this.home = home
    }

    new {
        label Str
    } = new(label, Point { x: 0, y: 0 })

    def moved(dx Int, dy Int) User = User {
        name: this.name
        home: Point {
            x: this.home.x + dx
            y: this.home.y + dy
        }
    }
}

def main() Unit {
    user User = User { label: "Ada" }
    moved = user.moved(3, 4)

    let { name, home } = moved
    println(name, home.x, home.y)
}
```

More runnable examples live in [examples/](examples/).

## Repository Layout

- `rust/` - Rust compiler, resolver, typechecker, lowering, interpreter, and CLI.
- `examples/` - Lume programs used as samples and parity fixtures.
- `stdlib/` - Standard library sources loaded by the Rust toolchain.
- `vscode-extension/` - Editor highlighting support.
- `syntax.md` - Current language syntax reference.
- `features.md` - Unsettled feature notes and design discussion.
- `ARCHITECTURE.md` - High-level architecture notes for the current pipeline.
