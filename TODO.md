# TODO

## Builtin Descriptor Follow-Ups

- Revisit `Set[T]` / `Map[K, V]` collection API breadth.
  - `iterator`, `map`, `flatMap`, `filter`, `fold`, `reduce`, `exists`, `forAll`, and `forEach` now exist in `stdlib/predef` and are wired through the runtime.

- Revisit `Array[T]` collection API shape.
  - `map`, `exists`, `forAll`, and `forEach` now exist and fit fixed-size arrays reasonably well.
  - `flatMap`, `filter`, `zip`, and `zipWithIndex` still need more thought.
  - In particular, `zipWithIndex` producing another `Array[...]` may be the wrong result shape for a fixed-size collection surface.
  - Decide whether those APIs should instead return `List[...]`, `Iterable[...]`, or be omitted from `Array` entirely.

## Enum Follow-Ups

- Think through auto-generated constant values for enum-wide fields.
  - Candidate syntax:

```txt
enum MyConstant {
    someId Int = 1++

    case Constant1
    case Constant2
}
```

  - Open questions:
    - whether `1++` is the right syntax, or whether another explicit auto-increment marker would read better
    - whether the generated values should be exposed through a built-in property like `ordinal` instead of a user-declared field
    - whether explicit overrides should be allowed in the same enum
    - how this should interact with non-`Int` enum-wide fields

## Syntax Follow-Ups

- Consider block-style trailing lambda syntax for call sites that take a function parameter.
  - Example target shape:

```txt
def fun((x Int, y Int) -> Int)

fun { x, y ->
    x + y
}
```

  - Main question: whether this reads as a natural extension of the current lambda syntax, or adds too much overlap with block expressions and existing `fun(x -> ...)` / `fun((x, y) -> ...)` call forms.

- Keep tuple conversion separate from anonymous-record conversion.
  - Current intended rule:
    - class/record -> tuple is not implicit
  - Open question:
    - whether to add an explicit `tuple(instance)` construct later for class/record -> tuple projection
    - whether anonymous record -> tuple should remain unsupported, or use the same explicit `tuple(instance)` surface later

- Consider explicit anonymous-record-to-class conversion syntax such as `anon as User`.
  - Goal:
    - keep `Type { ... }` as construction syntax
    - avoid treating `Type(class { ... })` or `Type(recordValue)` as implicit nominal conversion

- Consider an explicit assertive pattern-binding surface such as `expect Some(a) = optionOfA`.
  - Intended meaning:
    - match and bind on success
    - panic if the pattern does not match
  - Current state:
    - plain pattern `let` already has that runtime behavior
    - this would only make the intent more explicit in source
