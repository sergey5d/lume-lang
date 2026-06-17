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

Likely missing methods:
- clearer `Map` indexing ergonomics:
  - `map[key]` should likely act as lookup and return `Option[V]`
  - `map[key] := value` should likely act as set/update
  - this is intentionally different from list/array indexing, where `[]` may still return the element directly
- maybe collection partitioning helpers
- maybe zip / unzip style helpers later

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
- objects do not participate
- top-level functions do not participate

### 6. Module / Visibility Polish

Current module/import support is usable.

Settled direction:
- top-level bindings are private by default
- top-level functions are private by default
- exported top-level functions and exported immutable module bindings use explicit `public`
- mutable module state should not be exposed directly as imported variables

Still open:
- whether object members should ever be importable directly, for example importing `OS.println`-style names without importing the whole object surface
- if both a wide module import and a renamed selective import target the same module, the wide import should come first and the `as` import should come after it

## Longer-OS Ideas

### 9. Result / Either Style Error Values

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

### 10. Smarter Type Narrowing

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

### 11. Deferred Cleanup

A Go-like `defer` construct is still a possible future feature.

Potential shape:
- `defer close()`
- `defer { cleanup() }`

Main use cases:
- resource cleanup
- structured teardown
- keeping setup and cleanup close together in imperative code

Open questions:
- whether it should run at function exit only
- whether it should support block scope
- how it should interact with `return`, `break`, and runtime errors

## TBD

### `impl` Blocks For Methods

Top-level `impl Type { ... }` blocks exist now for attaching methods to classes, records, and enums, but the language still needs a final decision on whether `impl` should remain required for ordinary methods.

Open question:
- keep `impl Type { ... }` as the required home for methods on classes/records/enums
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
- class -> anonymous record is allowed implicitly
- tuple -> anonymous record is not allowed
- anonymous record -> class should be allowed when the compiler can lower it into constructor-style code
  - either a matching constructor exists
  - or the target class has only public fields, with any private fields already initialized
- class -> tuple should stay explicit, if added at all

Important separation:
- value conversion is a different design area from pattern destructuring
- allowing class values to convert to anonymous records does not automatically mean `match` should destructure them using anonymous-record-shaped patterns

Open questions:
- whether anonymous record -> class should be contextual-only based on the expected type
- how strict constructor matching should be
- whether anonymous record -> tuple should exist at all
- anonymous records use brace construction for both explicit named fields and contextual positional values
  - `class { count = count, label = label }`
  - positional brace construction like `class { 1, "x" }` is also allowed when a target anonymous-record shape is known from context
- whether explicit tuple projection should use a builtin like `tuple(instance)`

### Record Binding From String Templates

One possible future feature is record binding from interpolated-looking string templates, where a template shape declares the fields to extract.

Example direction:

```txt
record1 = "$time_local-$agent"
```

Possible meaning:
- produce something like `class { time_local Str, agent Str }`
- use the template itself as the binding/extraction surface

Possible larger example:

```txt
"$remote_addr - $remote_user [$time_local] \"$request\" $status $body_bytes_sent \"$http_referer\" \"$http_user_agent\""
```

This could be useful for:
- log parsing
- simple structured extraction from line-oriented text
- lightweight record creation without repeating field names separately

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
- today placeholder-based forms like `list.map(match _ { ... })` work
- possible future shorthand: allow implicit-input match lambdas without `_`
  - block form: `list.map(match { ... })`
  - single-expression shorthand form: `list.map(match: Some(x) => x + 1)`
- this would only make sense in contexts where a one-argument lambda is expected
- open question:
  - is this worthwhile readability improvement
  - or unnecessary contextual magic compared to the explicit `match _ { ... }` form

### Constructor / Companion Design

These are still open design options that need a decision.

Context:
- keep autogenerated primary constructors for the common case
- use same-named objects as privileged companions
- companions may access private members of their class when construction requires it

Open options under discussion:

1. Keep autogenerated constructors and add a way to make the generated primary constructor private
   Possible shape:
   - `private def new(*) { ... }`

2. Same as above, but using `...` instead of `*`
   Possible shape:
   - `private def new(...) { ... }`
   Concern:
   - `...` already means variadic parameters, so this may be misleading

3. Remove user-declared constructors entirely and rely on:
   - autogenerated primary constructors when fields allow them
   - companion object factories when a class has private uninitialized members

4. If a class defines a custom secondary construction path through its companion object, suppress the autogenerated primary constructor
   Concern:
   - this may be too implicit and surprising during refactors

Current leaning:
- autogenerated constructors still make sense
- companion objects are the special construction path for classes that cannot expose a normal public autogenerated constructor
- if explicit control is needed, `private def new(*) { ... }` currently looks clearer than the `...` variant

## Suggested Priority Order

1. `match`
2. enum + pattern ergonomics
3. derived `Eq` / `Hashed`
4. stdlib collection/query growth
5. operator overloading
6. anonymous objects
