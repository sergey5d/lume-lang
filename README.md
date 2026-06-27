# Lume

`Lume` is an experimental programming language with a Rust implementation in
this repository. The language is intentionally direct: classes are nominal,
shapes are structural data, methods live in `impl`, control flow is expression
friendly where it stays readable, and the runtime currently executes the Rust
compiler/interpreter pipeline under `rust/`.

The repo currently centers on:

- the Rust compiler/interpreter pipeline under `rust/`
- language docs such as `syntax.md` and `features.md`
- checked-in `examples/` and `stdlib/` source files
- editor support under `vscode-extension/`

For implementation details, start with [rust/README.md](rust/README.md). For the
full language reference, start with [syntax.md](syntax.md).

## Quick Start

From the repository root:

```bash
cargo run --manifest-path rust/Cargo.toml -p lume -- parse examples/constructors.lum
cargo run --manifest-path rust/Cargo.toml -p lume -- check examples/random_code/asset_prices.lum
cargo run --manifest-path rust/Cargo.toml -p lume -- run examples/range.lum
```

Run the Rust test suite:

```bash
cargo test --manifest-path rust/Cargo.toml -p lume
```

Run the checked example sweep:

```bash
./run_samples.sh
```

## Syntax Snapshot

- Use declarations use `use`, for example `use model/things/{A, B as AliasB}`.
- Classes construct nominal values with `Type(...)` for positional args and `Type { field: value }` for named args.
- Anonymous shapes use `{ field: value }`; named `shape` declarations are structural data-only types.
- Shapes can implement interfaces with `shape Name with Interface`; classes must be explicitly viewed as a shape before using shape-provided interfaces.
- Constructor declarations live in `impl` blocks as `new { params } { body }` or `new { params } = expr`.
- Variadic params use list type syntax: `items [Str] vararg`; constructor varargs can also be passed by name as a list.
- Field construction and anonymous-shape construction use `field: value`, not `field = value`.
- Assignment uses `=` for first binding / constructor field initialization and `:=` for reassignment.
- Enum payload cases use positional or named constructor syntax, for example `Status.Ready(3)` or `Status.Ready { value: 3 }`; zero-payload cases are bare, for example `Status.Empty`.
- `let`, `expect`, and `if let` support `<-` extraction from `Option`, `Result`, and `Either`.
- `match` is exhaustive; `partial match` returns `Option[...]`.
- `for`, `for ... yield`, `while`, `break`, `continue`, and callable-scoped `defer` are supported.
- Lambdas use explicit arrows: `value -> value + 1`, `(value Int) -> value + 1`, or trailing-call form `items.map { value -> ... }`.

Common import forms:

```txt
use model/things
use model/things/*
use model/things/A
use model/things/A as AliasA
use model/things/{A, B as AliasB}
use model/things/Console/{print as write}
```

## Example

This longer example is a compact tour of the current surface syntax: interfaces,
annotations, shapes, classes, `impl`, `single`, enums, constructors, varargs, named and
positional construction, destructuring, `let` / `expect`, `try`, `match`,
`partial match`, lambdas, `for`, `for ... yield`, `while`, `continue`, `defer`,
shape update, and shape merge.

```txt
interface Named {
    def label() Str
}

shape Point {
    x Int
    y Int
}

impl Point {
    def move(dx Int, dy Int) Point = Point {
        x: this.x + dx
        y: this.y + dy
    }
}

enum Status {
    case Empty
    case Ready {
        count Int
    }
    case Failed {
        reason Str
    }
}

single Log {
    prefix Str = "lume"
}

impl single Log {
    def headline(title Str) Unit = println(this.prefix, title)
}

class Project with Named {
    name Str
    origin Point
    tags [Str]
    hidden var visits Int = 0
}

impl Project {
    new {
        name Str
        origin Point
        tags [Str] vararg
    } {
        this.name = name
        this.origin = origin
        this.tags = tags
        this.visits = 0
    }

    def label() Str = this.name + "@" + this.origin.x + "," + this.origin.y

    def visit() Int {
        this.visits := this.visits + 1
        this.visits
    }

    def tagCount() Int = this.tags.size()

    def status() Status = if this.tags.size() == 0 {
        Status.Empty
    } else {
        Status.Ready(this.tags.size())
    }
}

def readyCount(status Status) Option[Int] = partial match status {
    case Ready(count) => count
}

def describe(status Status) Str = match status {
    case Status.Empty => "empty"
    case Ready(count) => "ready " + count
    case Failed(reason) => "failed " + reason
}

def firstTagOr(project Project, fallback Str) Str {
    let tag <- project.tags.get(0) else return fallback
    tag
}

def secondTag(project Project) Option[Str] {
    tag = try project.tags.get(1)
    Some(tag)
}

def report(project Project) Unit {
    defer println("done", project.name)
    let { name as projectName, origin } = project
    let { x, y } = origin
    println("project", projectName, x, y, project.label())
}

def main() Unit {
    Log.headline("syntax tour")

    project = Project("Lume", Point(3, 4), "parser", "runtime", "docs")
    named = Project { name: "Named", origin: Point(1, 2), tags: ["docs"] }
    empty = Project { name: "Empty", origin: Point(0, 0) }

    report(project)
    report(named)
    println("empty", describe(empty.status()))

    expect project.tagCount() == 3
    expect first <- project.tags.get(0)
    println("first", first)
    println("fallback", firstTagOr(empty, "none"))

    let Some(second) = secondTag(project) else return ()
    println("second", second)

    if let count <- readyCount(project.status()) {
        println("ready-count", count)
    }

    moved = project.origin.move(10, 20)
    updated = moved :< { y: 99 }
    meta = { owner: "core" } :+ { priority: 1 }
    println("point", updated.x, updated.y, meta.owner, meta.priority)

    doubled = [1, 2, 3].map(value -> value * 2)
    for (value, index) <- doubled.zipWithIndex() {
        println("doubled", index, value)
    }

    yielded = for {
        left <- [1, 2]
        right <- [10, 20]
    } yield left + right

    var total Int = 0
    for value <- yielded {
        if value == 22 {
            continue
        }
        total := total + value
    }

    var countdown Int = 3
    while countdown > 0 {
        total := total + countdown
        countdown := countdown - 1
    }

    println("total", total, project.visit())
}
```

## Repository Layout

- `rust/`
  Rust implementation, CLI, lowering, runtime, and tests.
- `examples/`
  Lume programs used as samples and parity fixtures.
- `stdlib/`
  Standard library sources loaded by the Rust toolchain.
- `syntax.md`
  Language syntax reference.
- `features.md`
  Feature notes and design status.
- `ARCHITECTURE.md`
  High-level architecture notes for the current Rust pipeline.
