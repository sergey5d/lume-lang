# Match Problems

This note captures the main remaining work for `match` after nested patterns,
guards, `partial`, and generic type-pattern erasure landed.

## 1. Unreachable-Case Detection

The checker handles basic exhaustiveness, but it still does not report
unreachable later cases.

Examples:

- `_` first, then more specific cases after it
- `SomeX { value: _ }` before `SomeX { value as x } if ...`
- duplicate case coverage that makes later branches impossible

This would make diagnostics much better and help `match` feel more complete.

## 2. Deeper Exhaustiveness

Enum exhaustiveness exists, but it is still fairly shallow.

Remaining work:

- nested enum exhaustiveness
- finite-domain tuple exhaustiveness
- stronger missing-case reporting
- better interaction with guards and richer pattern forms

Guards should probably continue to not contribute coverage, because that keeps
the totality model simple and predictable.

## 3. Generic Type-Pattern Policy

This part is now mostly settled:

- extractor patterns are statically generic-aware
- runtime type patterns are erased
- generic arguments inside runtime type patterns are rejected

What still remains is mostly documentation and examples, so the rule feels
deliberate rather than incidental.

## 4. Pattern-Lambda Sugar

Core `match` works inside explicit lambdas:

```txt
list.map(value -> match value {
    case SomeX { value as x } => x + 1
    case NoneX => 0
})
```

Possible later shorthand, if we ever want contextual lambda sugar:

```txt
list.map(match {
    case SomeX { value as x } => x + 1
    case NoneX => 0
})
```

This is not a correctness blocker, but it is still open ergonomics work.

## 5. Current Settled Surface

The main user-facing match story is now:

- `match` is exhaustive / total
- `partial` is the partial form and returns `Option[...]`
- guards are supported on top-level cases
- nested enum, tuple, class, and shape patterns are supported
- named payloads use name-based record patterns; only tuples are positional
- plain `match` should not fall back to runtime "no match" behavior
