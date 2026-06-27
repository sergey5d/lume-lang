# TODO

## Builtin Descriptor Follow-Ups

- Revisit `Set[T]` / `Map[K, V]` collection API breadth.
  - `iterator`, `map`, `flatMap`, `filter`, `fold`, `reduce`, `exists`, `forAll`, and `forEach` now exist in stdlib interfaces and are wired through the runtime.

- Revisit `Array[T]` collection API shape.
  - `map`, `exists`, `forAll`, and `forEach` now exist and fit fixed-size arrays reasonably well.
  - `zip` and `zipWithIndex` exist, but their fixed-size result shape still deserves scrutiny.
  - `flatMap` and `filter` still need more thought.
  - Decide whether those APIs should return `List[...]`, `Iterable[...]`, fixed-size `Array[...]`, or be omitted from `Array` entirely.

## Enum Follow-Ups

- Think through auto-generated constant values for enum-wide fields.
  - Candidate syntax:

```txt
enum MyConstant {
    someId Int = auto

    case Constant1
    case Constant2
}
```

  - Open questions:
    - whether `auto` is the right marker, or whether another explicit auto-increment marker would read better
    - whether the generated values should be exposed through a built-in property like `ordinal` instead of a user-declared field
    - whether explicit overrides should be allowed in the same enum
    - how this should interact with non-`Int` enum-wide fields

## Syntax Follow-Ups

- Consider explicit tuple projection syntax.
  - Settled rule:
    - tuple -> known shape is allowed
    - class/shape -> tuple is not implicit
  - Open question:
    - whether to add an explicit `tuple(instance)` construct later for class/shape projection

- Consider explicit shape-to-class conversion syntax such as `anon as User`.
  - Goal:
    - keep class construction nominal through `Type { ... }` and `Type(...)`
    - avoid treating a matching shape or tuple as an implicit class value

- Consider optional trailing messages for `expect` via comma syntax.
  - Candidate shapes:

```txt
expect split.size() == 3, "asset must have 3 parts"
expect Some(value) = maybeValue, "missing value"
expect {
    Some(left) = maybeLeft
    Some(right) = maybeRight
}, "missing input"
```

  - Goal:
    - keep the current statement-style `expect` syntax
    - support the same message form for boolean and pattern `expect`
  - Open question:
    - whether the trailing message should be restricted to `Str`, or allow any value renderable to `Str`
