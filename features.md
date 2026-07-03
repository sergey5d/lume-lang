# Feature Notes

This file captures the main language gaps and near-term design directions.
It is for unsettled or proposed features only; settled syntax and behavior
belong in `syntax.md`.

## Highest Priority

### 1. Type Pattern Analysis

Open checker/runtime work:
- generic type-aware type patterns at runtime
- unreachable-case detection

## Important Next Tier

### 2. Enum Ergonomics

Enums exist, but they still want:
- better generic type-pattern ergonomics

The biggest remaining enum work is more expressive generic type-pattern support.

Possible later enum constant/ordinal support:
- enum-wide fields could eventually have generated constant values
- candidate direction:

```txt
enum MyConstant {
    someId Int = auto

    case Constant1
    case Constant2
}
```

Open questions:
- whether `auto` is the right marker, or whether another explicit auto-increment marker would read better
- whether generated values should instead be exposed through a built-in property like `ordinal`
- whether explicit overrides should be allowed in the same enum
- how generated values should interact with non-`Int` enum-wide fields

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
- whether `Array[T]` should grow `flatMap` and `filter`; if yes, decide whether those return `List[...]`, `Iterable[...]`, fixed-size `Array[...]`, or should stay omitted

These can mostly live in the stdlib, but may still need runtime support in places.

Shape/object construction ergonomics to consider:
- field spread exists for anonymous-shape creation and update-like literals
- open question: field spread could eventually replace the dedicated shape update and merge operators `:<` and `:+`
- current anonymous-shape spread syntax:

```txt
updated = {
    ...existingShape
    age: 42
}

merged = {
    ...leftShape
    ...rightShape
}
```

Open questions:
- whether spread should be extended to named construction fields, for example `User { ...anon }`
- whether `User { ...anon }` should construct only when `anon` exactly matches the accepted constructor shape, or whether extra fields may be ignored
- whether spread should fully replace `:<` and `:+`, or coexist for a transition period

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
- use ordinary callable body syntax: `def ... = expr` or `def ... { ... }`
- do not introduce operator-specific body forms

Finalized policy:
- operator overloading is limited to interfaces, classes, and enums
- singles do not participate
- top-level functions do not participate

### 6. Module / Visibility Polish

Current module/use support is usable.

Settled direction:
- top-level functions are exported by default
- top-level immutable bindings are exported by default
- top-level types are exported by default
- `hidden` makes top-level functions, constants, and types private
- top-level mutable bindings are not allowed; mutable module state belongs inside `single`, class instances, or function locals

Still open:
- whether singleton methods should ever be usable directly beyond explicit `use module/Single/*` and builtin `OS` prelude behavior
- if both a wide module use and a renamed selective use target the same module, the wide use should come first and the `as` use should come after it

### 7. Annotation Targets

Annotations exist as compile-time metadata shapes. They currently attach to declarations and members, with annotation arguments restricted to literals, stable constants, and constant expressions.

Open question:
- do we want annotations on global functions/method-like top-level `def` declarations as a first-class supported target
- do we want annotations on global variables/top-level bindings
- if top-level bindings become annotatable, immutable constants are the only viable target because top-level mutable bindings are not allowed
- whether annotated globals should affect module export/import metadata, runtime reflection, generated code, or only checker/tooling behavior

Current leaning:
- global functions are probably useful annotation targets for routing, tests, effects, permissions, and generated bindings
- immutable top-level constants may be useful too, but annotation metadata should describe stable declarations, not changing state

## Longer-Term Ideas

### 8. Result / Either Style Error Values

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
- `Result[..., SourceE]` can propagate only into `Result[..., TargetE]` when `SourceE` is assignable to `TargetE`
- `Either[SourceL, ...]` can propagate only into `Either[TargetL, ...]` when `SourceL` is assignable to `TargetL`
- wrapper-style error remapping during `try` is still not implemented

### 9. Smarter Type Narrowing

Later improvements could include:
- better narrowing after `is`
- exhaustiveness analysis
- unreachable branch detection

Concrete example of the kind of narrowing worth considering:

```txt
if x is Str {
    println(x.size())
}
```

Meaning:
- after the `is Str` check succeeds, `x` would be treated as `Str` inside the `if` body
- the programmer would not need to write an explicit cast before using string-specific members

Possible follow-up extensions if this direction is adopted:
- narrowing in the `else` branch to mean "not that type"
- preserving narrowing after early exits, for example:

```txt
if !(x is Str) {
    return
}

println(x.size())
```

- combining narrowing with boolean conditions when the flow stays obvious

Main design question:
- whether this should stay very local and conservative
- or whether the checker should learn more control-flow-sensitive narrowing over time

### 10. Runtime Type Parameters

Do not rush to add reified generic syntax. Since the language now has `Type[A]`,
the core model should stay explicit first:

```txt
def load[A](ty Type[A], id Str) Option[A] {
    ...
}

user = load(typeOf[User], "123")
```

This is clear, teachable, and fits the language direction: fewer alternate
forms, explicit behavior, and less hidden compiler magic.

Runtime type evidence is useful mostly for library-style code:
- decoding JSON into `A`
- loading `A` from a registry
- reflecting over fields or annotations on `A`
- constructing default values for `A`
- mapping database rows into `A`

Most generic functions do not need runtime type metadata:

```txt
def id[A](value A) A = value
def first[A](items [A]) Option[A] = items.get(0)
def map[A, B](items [A], f (A) -> B) [B] = ...
```

Recommendation:
- now: require explicit `Type[A]` parameters when runtime type information is needed
- later: consider `[runtime A]` as sugar over a hidden `Type[A]` parameter
- never: reify all generic parameters automatically

Possible future sugar:

```txt
def load[runtime A](id Str) Option[A]
# or
def load[reified A](id Str) Option[A]
```

Lowering could be roughly:

```txt
def load[A](__typeA Type[A], id Str) Option[A]
```

Then call sites could become:

```txt
user = load[User]("123")
user Option[User] = load("123")
```

Only add this if explicit calls like `load(typeOf[User], id)`,
`decode(typeOf[Order], json)`, and `schema(typeOf[Route])` become noisy in real
examples.

Naming is unsettled:
- `runtime A` is plain and describes the behavior directly
- `reified A` is compiler-jargony, but that weirdness may be useful because it marks this as a specific type-system feature rather than a general runtime mode

Do not settle the keyword until explicit `Type[A]` examples show whether the
sugar is needed at all.

Useful diagnostic if someone tries to use erased generic type info directly:

```txt
generic type 'A' is not available at runtime;
pass a Type[A] parameter, for example `def bad[A](ty Type[A])`,
or mark A as reified/runtime if that feature is enabled
```

### 11. Deferred Cleanup

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

### 12. Flow Control Composition

Possible future surface:

```txt
first = flow if passedInTry == tryTime {
    passedOther = await(control.side.reverse(), 1)
}

second = flow(passedOther) if passedOther == 1 {
    control.inc(passedOther)
} else {
    set_light()
    control.reverseAndZero()
}

program = first & second
```

Idea:
- `flow if condition { ... }` captures a conditional control-flow fragment as a value
- `flow(value) if condition { ... } else { ... }` can depend on a value produced by another flow
- `&` composes flow fragments so a program can be assembled from smaller branching/control pieces

Open questions:
- whether `flow` is executable syntax, an IR-building DSL, or only a proposal-level abstraction
- what type `flow` expressions produce
- how values produced inside one flow become visible to later composed flows
- whether `&` should sequence unconditionally, short-circuit, or model dependency composition

### 13. Explicit Tuple Projection

Settled rule:
- tuple to known shape is allowed when the target shape is known
- class/shape to tuple is not implicit

Possible later syntax:

```txt
pair = tuple(instance)
```

Open question:
- whether an explicit `tuple(instance)` projection is useful enough to justify reintroducing positional views over named data
- if added, whether it should expose only visible fields and whether field order should be declaration order

## TBD

### Irrefutable and Refutable Binding

Binding syntax is organized around whether the pattern is irrefutable or
refutable:
- `let PATTERN = value` for irrefutable binding
- `if let PATTERN = value { ... }` for testing and binding with a branch
- `if let { PATTERN = value ... } { ... }` for multiple sequential refutable bindings in one condition
- `if let PATTERN = value && let OTHER = next && ready { ... }` for mixed refutable and boolean checks in one condition
- `let PATTERN = value else { ... }` for refutable binding with an explicit failure path
- `let { PATTERN = value ... } else { ... }` for multiple sequential refutable bindings sharing one fallback
- `expect PATTERN = value` for assertive refutable binding that panics on mismatch; if the binding is irrefutable, use `let`
- `assert(condition)` and `assert(condition, message)` for boolean assertions that panic when false
- `PATTERN <- source` as shorthand for the success case inside `if let`, `let ... else`, and `expect`
  `Some(PATTERN)` for `Option`, `Ok(PATTERN)` for `Result`, and `Right(PATTERN)` for `Either`
  plain `let` accepts this form only when the source expression itself proves the success case, such as `let item <- Some(5)`
- `value = try source` for propagation from `Option`, `Result`, and `Either`

Open follow-ups:
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

### Anonymous-Shape Binding From String Templates

One possible future feature is anonymous-shape binding from interpolated-looking string templates, where a template shape declares the fields to extract.

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
- whether the result should be an anonymous shape or a named type when a context type is available
- whether failure should produce `Option`, `Result`, or a runtime error

### Singleton Factory Questions

- whether same-named `single` declarations should act as privileged factory companions
- whether singleton factory methods should ever get hidden-field access to class internals

Current leaning:
- `single` factory methods are useful as ordinary namespaced helpers, but hidden-field companion privileges should be a separate deliberate decision

## Suggested Priority Order

1. enum + pattern ergonomics
2. derived `Eq` / `Hashed`
3. stdlib collection/query growth
4. operator overloading
