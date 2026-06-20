# Feature Notes

This file captures the main language gaps and near-term design directions.

## Highest Priority

### 1. Match / Pattern Matching

`match` exists now, including:
- enum case patterns
- tuple patterns
- literal/value patterns
- class extractor patterns
- simple type patterns
- guards
- nested enum / tuple / class extractor patterns

Still missing:
- generic type-aware type patterns at runtime
- unreachable-case detection

Current matching split:
- `match` is the exhaustive / total form
- `partial` is the partial form and returns `Option[...]`

## Important Next Tier

### 2. Enum Ergonomics

Enums exist, but they still want:
- better generic type-pattern ergonomics

Now that `match` exists and enum exhaustiveness is checked, the biggest remaining enum work is more expressive generic type-pattern support.

### 3. Derived Protocols

Classes should eventually support auto-derived protocols when they stay value-like.

Likely targets:
- `Eq`
- `Hashed` when all fields are hashable
- maybe `Show` / `Stringify` later

This reduces boilerplate and helps stdlib types feel native.

### 4. Collection / Query APIs

The language now has `for ... yield`, `map`, `flatMap`, `filter`, `fold`, `reduce`, `exists`, `forAll`, and `forEach`, but stdlib collection ergonomics still need growth.

Current collection conveniences:
- `Map` indexing exists: `map[key]` acts as lookup and returns `Option[V]`
- `List` and `Array` have `zip` and `zipWithIndex`
- `List`, `Set`, and `Map` have broad higher-order method coverage

Likely remaining gaps:
- clearer `Map` update ergonomics beyond `put`
- maybe collection partitioning helpers
- maybe unzip style helpers later

These can mostly live in the stdlib, but may still need runtime support in places.

Construction direction:
- avoid `apply` call magic for collections and similar surfaces
- prefer explicit factory names like `of`, `from`, or `empty`
- when a type wants custom construction sugar, prefer descriptive helpers like `create`, `from`, or `make`

## Medium Priority

### 5. Operator Overloading

Operator overloading now exists and is mainly intended for compact value-oriented declared types such as:
- numeric-like wrappers
- vectors / matrices / geometry values
- class domain values like `Money`, `Distance`, or `Duration`
- interface-driven abstractions that want symbolic operators over implementing types

Constraint:
- keep it same-line only; no newline-based implicit body after `:`

Finalized policy:
- operator overloading is limited to interfaces, classes, and enums
- singles do not participate
- top-level functions do not participate

### 6. Module / Visibility Polish

Current module/use support is usable.

Settled direction:
- top-level bindings are private by default
- top-level functions are private by default
- exported top-level functions and exported immutable module bindings use explicit `public`
- mutable module state should not be exposed directly as used variables

Still open:
- whether singleton methods should ever be usable directly beyond explicit `use module/Single/*` and builtin `OS` prelude behavior
- if both a wide module use and a renamed selective use target the same module, the wide use should come first and the `as` use should come after it

## Longer-Term Ideas

### 7. Result / Either Style Error Values

`Option`, `Result`, `Either`, and `try`-based short-circuit propagation now exist.

Still open:
- whether `try`-style propagation should stay hardcoded to these builtins or later grow a broader protocol

Important design constraint:
- expressing "same container family, different success type" is hard without higher-kinded types
- so the current propagation model relies on compiler help for propagation checks

Clarification on "failure conversion":
- this does not necessarily mean superclass/subclass conversion
- the more likely model is wrapper-style conversion into a broader application error type
- example:
  - `readFile() Result[Str, IoError]`
  - enclosing function returns `Result[Int, AppError]`
  - failure conversion would mean allowing `IoError` to be turned into something like `AppError.Io(...)` during propagation

Current behavior:
- same-family propagation with a different success type is supported
- `Option[...]` can propagate only into `Option[...]`
- `Result[..., E]` can propagate only into `Result[..., E2]` when `E` is assignable to `E2`
- `Either[L, ...]` can propagate only into `Either[L2, ...]` when `L` is assignable to `L2`
- wrapper-style error remapping during `try` is still not implemented

### 8. Smarter Type Narrowing

Later improvements could include:
- better narrowing after `is`
- exhaustiveness analysis
- unreachable branch detection

Concrete example of the kind of narrowing worth considering:

```txt
if (x is String) {
    println(x.length)
}
```

Meaning:
- after the `is String` check succeeds, `x` would be treated as `String` inside the `if` body
- the programmer would not need to write an explicit cast before using string-specific members

Possible follow-up extensions if this direction is adopted:
- narrowing in the `else` branch to mean "not that type"
- preserving narrowing after early exits, for example:

```txt
if !(x is String) {
    return
}

println(x.length)
```

- combining narrowing with boolean conditions when the flow stays obvious

Main design question:
- whether this should stay very local and conservative
- or whether the checker should learn more control-flow-sensitive narrowing over time

### 9. Deferred Cleanup

`defer` is implemented as callable-scoped cleanup.

Supported shape:
- `defer close()`
- `defer { cleanup() }`

Current behavior:
- deferred actions run in LIFO order when the enclosing function, method, or lambda returns
- `defer` is not block-scoped
- lambdas have their own defer queue
- deferred blocks may not contain `return`, `break`, or `continue`

Main use cases:
- resource cleanup
- structured teardown
- keeping setup and cleanup close together in imperative code

Open questions:
- whether runtime errors should also run pending defers
- whether future async/concurrency features need a stronger cleanup model

## TBD

### `impl` Blocks For Methods

Top-level `impl Type { ... }` blocks exist now for attaching methods to classes and enums, and `impl single Name { ... }` attaches singleton methods. The language still needs a final decision on whether `impl` should remain required for ordinary methods.

Open question:
- keep `impl Type { ... }` as the required home for methods on classes/enums
- or allow methods inline in the original type declaration and treat `impl` as optional extra syntax

Current leaning:
- `impl` looks cleaner for medium and large types because it separates shape from behavior
- but it should probably remain optional rather than mandatory, because small types often read better when fields and methods stay together

### Single-Line Body Syntax

The shorthand body rules are now intentionally narrow:
- new code should prefer brace-delimited `if`
- `for` is block-only
- `match` and `partial` are block-only
- `else expr` and `yield expr` are valid same-line forms
- if a shorthand body moves to the next line, a `{ ... }` block is required

This keeps the surface compact without turning newlines into implicit structure.

### Refutable Binding

The preferred refutable-binding surface is now split into these forms:
- `if let PATTERN = value { ... }` for testing and binding
- `if let { PATTERN = value ... } { ... }` for multiple sequential refutable bindings in one condition
- `if let PATTERN = value && let OTHER = next && ready { ... }` for mixed refutable and boolean checks in one condition
- `let PATTERN = value else { ... }` for extraction with an explicit failure path
- `let { PATTERN = value ... } else { ... }` for multiple sequential refutable bindings sharing one fallback
- `PATTERN <- source` as shorthand for the success case inside `if let`, `let ... else`, and `expect`
  `Some(PATTERN)` for `Option`, `Ok(PATTERN)` for `Result`, and `Right(PATTERN)` for `Either`
  plain `let` does not accept this form; use `let ... else`, `if let`, or `expect`
- `value = try source` for propagation from `Option`, `Result`, and `Either`

TODO:
- consider direct nested payload destructuring inside `if let`, for example `if let Some((_, initialY, _)) = rows.get(0) { ... }`
- for now, prefer `if let Some(row) = rows.get(0) { ... }` and destructure `row` on the next line inside the branch
- keep `if let` chaining limited to `&&` joins; if we ever extend it, do that deliberately rather than broadening it implicitly through general boolean syntax

### Lambda Surface

The language currently uses arrow-based lambda syntax directly, for example:

```txt
x -> x + 1
(x, y) -> x + y
```

Open question:
- should lambda declarations stay keyword-free
- or should the language grow an explicit `lambda` keyword for some or all lambda forms

Possible motivations for revisiting this:
- making lambdas more visually explicit to new readers
- reducing ambiguity in more complex nested expressions
- giving room for future lambda-surface variants if the arrow-only form starts feeling overloaded

Current leaning:
- keep the current keyword-free arrow form unless real readability problems show up
- only add a `lambda` keyword if it solves a concrete ambiguity or makes larger expressions meaningfully clearer

### Product Type Conversion Surface

The language still needs a final policy for conversions between:
- classes
- anonymous records
- tuples

Current intended direction:
- class -> anonymous record shape is allowed where structural access is explicitly expected
- tuple -> anonymous record is not allowed
- anonymous record-shaped class construction should stay explicit through `Type { ... }`
  - the target class must not define an explicit `new`
  - the target class must have a valid visible structural shape
- class -> tuple should stay explicit, if added at all

Important separation:
- value conversion is a different design area from pattern destructuring
- allowing class values to convert to anonymous records does not automatically mean `match` should destructure them using anonymous-record-shaped patterns

Open questions:
- whether anonymous record -> class should be contextual-only based on the expected type
- how strict constructor matching should be
- whether anonymous record -> tuple should exist at all
- anonymous records use brace construction for both explicit named fields and contextual positional values
  - `{ count: count, label: label }`
  - positional brace construction like `{ 1, "x" }` is also allowed when a target anonymous-record shape is known from context
- whether explicit tuple projection should use a builtin like `tuple(instance)`

### Anonymous-Record Binding From String Templates

One possible future feature is anonymous-record binding from interpolated-looking string templates, where a template shape declares the fields to extract.

Example direction:

```txt
parsed = "$time_local-$agent"
```

Possible meaning:
- produce something like `{ time_local Str, agent Str }`
- use the template itself as the binding/extraction surface

Possible larger example:

```txt
"$remote_addr - $remote_user [$time_local] \"$request\" $status $body_bytes_sent \"$http_referer\" \"$http_user_agent\""
```

This could be useful for:
- log parsing
- simple structured extraction from line-oriented text
- lightweight structured value creation without repeating field names separately

Open questions:
- whether this should be a pure binding/extraction feature, a parser combinator surface, or just syntax sugar over regex/string parsing
- how field types would be inferred
- whether all extracted fields should default to `Str`
- how escaping and delimiter ambiguity should work
- whether the result should be an anonymous record or a named type when a context type is available
- whether failure should produce `Option`, `Result`, or a runtime error

### Match Totality / Partial Match Behavior

This is now settled:
- avoid runtime "no match" exceptions as a normal language outcome
- keep `match` as the exhaustive / total form
- partial matching now uses `partial`

Related lambda-syntax discussion:
- today explicit lambda forms like `list.map(value -> match value { ... })` work
- possible future shorthand: allow implicit-input match lambdas
  - block form: `list.map(match { ... })`
  - single-expression shorthand form: `list.map(match: Some(x) => x + 1)`
- this would only make sense in contexts where a one-argument lambda is expected
- open question:
  - is this worthwhile readability improvement
  - or unnecessary contextual magic compared to the explicit lambda form

### Constructor / Singleton Factory Design

The current constructor surface is:
- `def new(...)` declares explicit constructors
- `hidden def new()` can hide an explicit constructor
- any explicit `new` suppresses structural brace construction
- `Type { ... }` is structural construction when no explicit constructor blocks it
- `Type(...)` is reserved for explicit `new(...)` and builtin constructor forms

Still open:
- whether same-named `single` declarations should act as privileged factory companions
- whether singleton factory methods should ever get hidden-field access to class internals
- whether additional constructor-hiding syntax is needed beyond `hidden def new()`

Current leaning:
- autogenerated structural construction still makes sense for simple public shapes
- explicit `new` remains the escape hatch for custom construction
- `single` factory methods are useful as ordinary namespaced helpers, but hidden-field companion privileges should be a separate deliberate decision

## Suggested Priority Order

1. enum + pattern ergonomics
2. derived `Eq` / `Hashed`
3. stdlib collection/query growth
4. operator overloading
5. constructor/factory polish
