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

- Add `continue`.
  - Main questions:
    - whether `continue` should be valid in both `for` and `while`
    - whether `continue` inside `for ... yield` should be allowed at all
    - what the parser/runtime diagnostics should say when used outside a loop

- Consider block-style trailing lambda syntax for call sites that take a function parameter.
  - Example target shape:

```txt
def fun((x Int, y Int) -> Int)

fun { x, y ->
    x + y
}
```

  - Main question: whether this reads as a natural extension of the current lambda syntax, or adds too much overlap with block expressions and existing `fun(x -> ...)` / `fun((x, y) -> ...)` call forms.

- Settle anonymous-record conversion rules.
  - Current intended rules:
    - class/record -> anonymous record is allowed implicitly
    - tuple -> anonymous record is not allowed
    - anonymous record -> class/record should be allowed when generated constructor-calling code can be formed
      - either a matching constructor exists
      - or the target class has only public fields, with any private fields already initialized
  - Open questions:
    - whether anonymous record -> class/record should be contextual-only based on expected type
    - whether constructor matching should be exact by field names only, or also allow constructor parameter reordering by name
    - how much of this should happen purely in typechecking vs lowered/generated code

- Keep tuple conversion separate from anonymous-record conversion.
  - Current intended rule:
    - class/record -> tuple is not implicit
  - Open question:
    - whether to add an explicit `tuple(instance)` construct later for class/record -> tuple projection
    - whether anonymous record -> tuple should remain unsupported, or use the same explicit `tuple(instance)` surface later
