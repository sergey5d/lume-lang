# Syntax Cleanup Notes

This file is a lightweight readability watchlist. It is intentionally secondary
to `syntax.md`, which is the reference for what works today. Unsettled feature
work belongs in `features.md`.

Cleanup principle:

- prefer regularity over clever compactness
- keep punctuation meaningful
- avoid a second spelling unless it buys real readability

## Settled Cleanup

These older surfaces should stay out of new examples and docs:

- class-to-tuple destructuring; use brace destructuring
- anonymous-shape-to-class conversion; construct the class explicitly
- `OS.println(...)` in examples; use prelude `println(...)`

### Symbolic Operators

Operator overloading exists, but should stay conservative.

Good candidates:

- arithmetic-like value types
- vectors / matrices / geometry values
- domain values such as `Money`, `Distance`, or `Duration`

Risky candidates:

- symbolic forms that hide domain behavior
- aliases for ordinary named methods
- clever collection punctuation when `add`, `addAll`, `from`, or `put` reads better

### Lambda Surface

The keyword-free lambda surface is currently good:

```txt
x -> x + 1
(x, y) -> x + y
users.map(mapUser)
users.map(User.toDto)
```

Still worth watching:

- whether nested lambdas ever become visually noisy enough to justify a `lambda` keyword
- current leaning: do not add a keyword unless examples show clear readability pain


## Summary

Keep moving toward:

- fewer alternate forms
- less contextual sugar
- explicit behavior homes
- clear construction syntax
- explicit lambdas plus callable references for obvious forwarding cases
