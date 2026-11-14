# Lume Programming Language

Lume is an experimental, statically typed programming language. It aims to
combine the flexibility of TypeScript and the power of Java while keeping code
clear, concise, and easy to reason about.

The implementation is written in Rust and includes a compiler, interpreter,
Java source backend, a small standard library, checked examples, and VS Code
editing support.

Start with the [Lume documentation](https://sergey5d.github.io/lume-lang/) for an
overview and guided introduction. The complete language reference lives in
[SYNTAX.md](SYNTAX.md). For implementation details, see
[ARCHITECTURE.md](ARCHITECTURE.md) and [rust/README.md](rust/README.md).

## Design Goals

- **TypeScript-like flexibility.** Model data and behavior with structural
  shapes, nominal classes, unions, interfaces, and ordinary functions, rather
  than forcing every problem into one object model.
- **Java-like power.** Support real application development and access to the
  Java ecosystem without making everyday code carry unnecessary ceremony.
- **Clarity and lower mental load.** Keep data flow and control flow visible,
  rules locally predictable, and hidden behavior to a minimum. Reading code
  should not require reconstructing what the compiler might mean.
- **Concise and easy to learn.** Reduce boilerplate without replacing it with
  cryptic shortcuts. Favor a small set of consistent, composable concepts over
  special cases that must be memorized.

These are design goals, not a claim that the experimental implementation
already matches TypeScript's flexibility or Java's capabilities and maturity.

## Language Model

Lume separates the main concepts deliberately:

- `class` is the nominal runtime type: it owns identity, fields, visibility, and
  class construction.
- `shape` is structural data with visible, shallowly read-only field bindings:
  fields cannot be replaced, while referenced values retain their own
  mutability rules. Assignment preserves the complete field-name set; dropping
  fields requires explicit construction, such as `Point { ...original }` or
  `point Point = { ...original }`.
- `type Name = class ... | shape ... | object ...` declares a closed union of
  alternatives, while a standalone named `object` declares one shared object
  value with optional fields and methods. `object { ... }` creates an anonymous
  object value.
- `interface` describes behavior; types opt in explicitly instead of gaining
  behavior by accidental structural matches.

Construction is its own idea:

- `new(name Type, other Type = default)` declares the inputs accepted by a
  class constructor.
- `Type { field: value }` fills constructor inputs by field name.
- `Type(value)` fills explicit constructors, implicit visible-field constructors, or named shape fields by declaration order.
- `new { field }` forces ambiguous braces to be construction; it uses an
  expected concrete class or shape target when available and otherwise infers
  an anonymous shape from nonempty fields. Contextual class construction always
  requires `new`.
- `{ field: value }` constructs the expected shape when one exists and otherwise
  synthesizes an anonymous shape. It does not construct a class.
- `new(value)` requires an expected constructible target with a known positional
  input order. That target may be a class, a named shape, or an explicitly
  written anonymous shape schema.
- If a class has no explicit `new`, the compiler synthesizes field construction
  from visible fields.

Control flow is expression-friendly but still explicit:

- `match` is exhaustive; optional results are expressed explicitly with `Some` and `None`.
- `let` and `if let` are pattern-oriented binding forms;
  `assert(...)` handles boolean assertions.
- `try`, pattern bindings, `??`, and explicit `map`/`flatMap` calls handle
  `Option`, `Result`, and `Either` flow.

Numeric models stay distinct: `Int` is checked signed 64-bit arithmetic, `Float`
is binary64, and `Decimal` provides exact arbitrary-precision decimal arithmetic
with literals such as `19.99d` and explicit rounding. See
[decimal arithmetic](SYNTAX.md#decimal-arithmetic) and the runnable
[Decimal example](examples/features/decimal.lum).

## Quick Start

Run the CLI from the repository root:

```bash
cargo run --manifest-path rust/Cargo.toml -p lume -- parse examples/constructors.lum
cargo run --manifest-path rust/Cargo.toml -p lume -- fmt examples/constructors.lum
cargo run --manifest-path rust/Cargo.toml -p lume -- check examples/random_code/asset_prices.lum
cargo run --manifest-path rust/Cargo.toml -p lume -- run examples/range.lum
```

Run tests:

```bash
cargo test --manifest-path rust/Cargo.toml -p lume
./run_samples.sh
```

## A Complete Example

This small reporting pipeline loads typed JSON, validates and normalizes each
order, groups totals by customer, and prints a sorted report. A second input
shows the error path. All data is fictional and kept in memory; no database,
server, or credentials are needed.

<!-- readme-example: order-report -->
```lume
module examples/readme_order_report

use lume/json/{Json}

public shape Order {
    customer Str
    quantity Int
    unitPrice Decimal

    get total Decimal = quantity.toDecimal() * unitPrice
}

public shape OrderBatch {
    orders [Order]
}

shape CustomerSummary {
    customer Str
    orders Int = 0
    total Decimal = 0d
}

interface OrderSource {
    get name Str
    fn load() Result[[Order], Str]
}

class JsonOrderSource with OrderSource {
    name Str
    text Str

    fn load() Result[[Order], Str] {
        batch = try Json.decode[OrderBatch](text)
        Ok(batch.orders)
    }
}

fn validate(order Order) Result[Order, Str] {
    customer = order.customer.trim()
    if customer.isEmpty {
        return Err("customer is required")
    }
    if order.quantity <= 0 {
        return Err("quantity must be positive")
    }
    if order.unitPrice < 0d {
        return Err("unitPrice must be non-negative")
    }
    Ok(Order { ...order, customer })
}

fn summarize(source OrderSource) Result[[CustomerSummary], Str] {
    orders = try source.load()
    totals [Str: CustomerSummary] = []

    for let (order, index) <- orders.zipWithIndex() {
        clean = try validate(order).mapError(error => "order ${index + 1}: $error")
        previous = totals[clean.customer] ?? CustomerSummary { customer: clean.customer }
        totals[clean.customer] := CustomerSummary {
            ...previous
            orders: previous.orders + 1
            total: previous.total + clean.total
        }
    }

    rows = totals.values()
    rows.sort((left, right) => {
        if left.total == right.total {
            return left.customer.compareTo(right.customer)
        }
        right.total.compareTo(left.total)
    })
    Ok(rows)
}

fn printReport(source OrderSource) {
    match summarize(source) {
        case Ok { value as rows } => {
            println(source.name)
            for row <- rows {
                println("${row.customer}: orders=${row.orders}, total=${row.total}")
            }
        }
        case Err { error } => println("${source.name}: $error")
    }
}

fn main() {
    source OrderSource = JsonOrderSource {
        name: "Daily orders"
        text: raw"""{"orders": [
            {"customer": " Mira ", "quantity": 2, "unitPrice": 12.50},
            {"customer": "Noah", "quantity": 1, "unitPrice": 8.75},
            {"customer": "Mira", "quantity": 1, "unitPrice": 5.00}
        ]}"""
    }
    printReport(source)

    invalid OrderSource = JsonOrderSource {
        name: "Invalid orders"
        text: raw"""{"orders": [{"customer": "Mira", "quantity": 0, "unitPrice": 12.50}]}"""
    }
    printReport(invalid)
}
```

Output:

```text
Daily orders
Mira: orders=2, total=30.00
Noah: orders=1, total=8.75
Invalid orders: order 1: quantity must be positive
```

Run the [same example](examples/readme_order_report.lum) with the interpreter:

```bash
cargo run --manifest-path rust/Cargo.toml -p lume -- run examples/readme_order_report.lum
```

The example keeps a few distinctions visible:

- `OrderSource` describes behavior; `JsonOrderSource` supplies it. Its public
  `name` field satisfies the interface's getter without a forwarding method.
- The public `OrderBatch` and `Order` models are available to the JSON library.
  `get total` computes a value without adding another stored field, and
  `Order { ...order, customer }` constructs a normalized copy without changing
  the input.
- `try` propagates an error, `mapError` adds context, `??` supplies an absent
  map entry, and `match` handles the final result explicitly.
- `Decimal` preserves exact decimal amounts; `compareTo` states the intended
  ordering directly.

More runnable examples cover [stateful aggregation](examples/random_code/record_aggregator.lum),
[generic aliases](examples/features/generic_type_aliases.lum),
[mixins](examples/features/mixins.lum), [regular expressions](examples/features/regex_captures.lum),
and [Java-backed HTTP services](examples/java_gradle_rest/README.md).

## Repository Layout

- `rust/` - Rust compiler, resolver, typechecker, lowering, interpreter, and CLI.
- `examples/` - Lume programs used as samples and parity fixtures.
- `stdlib/` - Standard library sources loaded by the Rust toolchain.
- `vscode-extension/` - Editor highlighting, formatting, and type-definition navigation.
- `SYNTAX.md` - Current language syntax reference.
- `features.md` - Unsettled feature notes and design discussion.
- `ARCHITECTURE.md` - High-level architecture notes for the current pipeline.
