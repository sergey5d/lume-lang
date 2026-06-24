# Lume

`Lume` is an experimental programming language with a Rust implementation in this repository.

The repo currently centers on:

- the Rust compiler/interpreter pipeline under `rust/`
- language docs such as `syntax.md` and `features.md`
- checked-in `examples/` and `stdlib/` source files
- editor support under `vscode-extension/`

For implementation details, start with [rust/README.md](/Users/sergeyd/Projects/a-lang/rust/README.md).

## Quick Start

From the repository root:

```bash
cargo run --manifest-path rust/Cargo.toml -p lume -- parse hello.lum
cargo run --manifest-path rust/Cargo.toml -p lume -- check examples/import_forms.lum
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

## Example

```txt
class Counter {
    hidden var count Int
}

impl Counter {
    new {
        count Int
    } {
        this.count = count
    }

    def inc() Int {
        this.count += 1
        return this.count
    }
}

def main() Int {
    counter Counter = Counter { 1 }
    counter.inc()
}
```
