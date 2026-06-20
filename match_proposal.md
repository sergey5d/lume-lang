# Match Proposal

This note captures the current `match` direction and the remaining polish after
guards, nested patterns, and `partial` landed.

## Current Surface

The current supported surface includes:

- enum constructor patterns
- tuple patterns
- class extractor patterns
- nested enum constructor patterns
- nested tuple patterns
- nested class extractor patterns
- allow `_` in any nested position to omit fields
- guards attached to top-level cases
- type-pattern reasoning/checking stays top-level only
- do not support destructuring classes into tuples
- do not support destructuring classes into anonymous-record shapes yet
- guards do not contribute coverage
- nested singleton enum cases stay qualified, for example `Wrap(InnerFlag.On)`

Examples of supported shapes:

```txt
match value {
    case Some((x, y)) => ...
    case Box(Apple(a)) => ...
    case Pair(_, y) => ...
    case Some(x) if x > 10 => ...
}
```

Examples that are intentionally out of scope for now:

```txt
match value {
    case Some(x if x > 0) => ...        # nested guard
    case Person(name, age) as (x, y) => ...
    case Person({ name: x, age: y }) => ...
}
```

Future feature to discuss later:

- allow anonymous records to participate in destructuring patterns
  - for example matching a class against an anonymous-record-shaped pattern
  - this is explicitly not part of the first nested-pattern implementation

Clarification:

- this note is about pattern destructuring only
- expression-level value conversion rules are separate
- so even if class values later become compatible with anonymous-record-shaped expectations in ordinary expressions, that does not by itself imply anonymous-record-shaped `match` patterns should work

## Main Improvement Areas

### 1. Generic-Aware Type Patterns

Constructor and extractor patterns already carry substituted field types correctly, but type-pattern matching still needs a clearer generic story, especially for:

- generic classes behind interface-typed values
- generic enums behind wider typed values
- distinguishing `Box[Int]` from `Box[Str]` when the runtime currently does not preserve explicit type arguments on instances

This is now mostly about deciding whether generic type patterns should:

- behave with erased runtime semantics
- inspect payload or field values structurally where possible
- or preserve concrete type arguments on runtime instances

### 2. Unreachable-Case Detection

Examples:

- wildcard case first
- later specific case that can never run

This would improve diagnostics and make `match` feel more complete as a checked language feature.

Current target:

- obvious structural unreachable detection first
- no deep guard reasoning initially

### 3. Partial-Match Story

`partial` exists, so the main open question is whether that is the final shape.

Open questions:

- is `partial` the final partial-match syntax?
- should partial matching get better fallback ergonomics?

This is more about language-shape polish than basic capability.

### 4. Pattern-Lambda / Collection Ergonomics

This is a smaller refinement, but still useful.

Current explicit style:

```txt
list.map(value -> match value {
    case Some(x) => x + 1
    case None => 0
})
```

Possible later shorthand:

```txt
list.map(match {
    case Some(x) => x + 1
    case None => 0
})
```

This should come after the core `match` model is finished.

### 5. Exhaustiveness Depth

Enum exhaustiveness already exists in a basic form.

Possible next improvements:

- better reporting
- support with nested patterns
- more complete analysis across richer pattern shapes

Possible target:

- deeper exhaustiveness within a conservative finite-domain limit
- top-level type-pattern rules stay as they are now

## Suggested Priority

If we want `match` to feel finished, the best order is probably:

1. unreachable-case detection
2. better missing-case reporting
3. generic-aware extraction policy
4. partial-match polish
5. pattern-lambda sugar

That order gives the biggest practical readability gains first while keeping syntax churn lower.
