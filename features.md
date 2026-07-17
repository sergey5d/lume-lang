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

Enum follow-ups:
- better generic type-pattern ergonomics
- type-aware rejection of obsolete empty-payload calls in patterns, such as `Option.None()` in `match`; bare cases like `None` should be the only valid form

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

Open stdlib collection/query ergonomics:
- clearer `Map` update ergonomics beyond `put`
- maybe collection partitioning helpers
- maybe unzip style helpers later
- whether `Array[T]` should grow `flatMap` and `filter`; if yes, decide whether those return `List[...]`, `Iterable[...]`, fixed-size `Array[...]`, or should stay omitted

These can mostly live in the stdlib, but may still need runtime support in places.

Open shape/construction ergonomics:
- whether spread should be extended to named construction fields, for example `User { ...anon }`
- whether `User { ...anon }` should construct only when `anon` exactly matches the accepted constructor shape, or whether extra fields may be ignored

Open construction helper naming direction:
- whether collection/custom construction helpers should consistently prefer names like `of`, `from`, `empty`, `create`, or `make`
- whether any collection-like type deserves special construction sugar, or whether descriptive factory names are enough

## Medium Priority

### 5. Word Equality Operators

Open equality spelling question:
- consider adding word operators `eq` and `neq` as readable aliases for `==` and `!=`
- `left eq right` would mean exactly `left == right`
- `left neq right` would mean exactly `left != right`
- if added, they should share equality precedence and lowering rather than becoming separately overloadable operators
- decide whether this improves readability enough to justify another spelling for equality

### 6. Module / Visibility Polish

Open module/use questions:
- whether singleton methods should ever be usable directly beyond explicit `use module/Single/*` and builtin `OS` prelude behavior
- if both a wide module use and a renamed selective use target the same module, the wide use should come first and the `as` use should come after it
- decide how enum cases are imported: if `EnumA` is imported, should users write `EnumA.CaseA`, or should `CaseA` also become directly available
- decide whether extension-method imports should keep using ordinary wildcard module use, or get a dedicated import surface such as `use ext app/module/*` or `use ext app/module/TypeName`
- decide whether extension methods should also be allowed on `single` types, or whether `impl single Name { ... }` is enough and keeps singleton behavior explicit

### 7. Interface Method Conflict Resolution

Open checker/runtime design:
- decide how method resolution should work when a type implements multiple interfaces that inherit or declare conflicting method signatures
- decide whether a class/type implementation can explicitly call a specific inherited interface/default method implementation from inside its own method body
- choose syntax for qualified interface dispatch if needed
- possible call forms to compare:
  - `super.method(args)` if there is exactly one unambiguous inherited implementation
  - `InterfaceName.method(this, args)` as an explicit static-looking dispatch form
  - `InterfaceName.super.method(args)` as a Java-like qualified-super form
  - another Lume-specific form if the above read too foreign or imply the wrong object model
- define diagnostics for ambiguous interface method calls so the compiler points users toward the disambiguation syntax

Related syntax question:
- whether explicit implementation/override markers would add enough readability in large types to justify extra syntax
- if added, decide whether they should mark interface satisfaction, override of a concrete method, or both
- define diagnostics for accidental signature mismatches even if no marker is added

### 8. Function Type Variance

Open checker work:
- make sure function/lambda type assignability follows the usual variance rule
- parameter types are contravariant
- return types are covariant

Example:

```txt
# If Dog <: Animal, then this is safe:
expected (Dog) -> Animal = (animal Animal) -> Dog()
```

Reason:
- the assigned function can accept at least every argument the caller may pass
- the assigned function returns a value no wider than the expected return type

This is the same core rule used by Scala function types such as
`Function1[-A, +B]`. Java does not have first-class function types in quite the
same way, but functional-interface APIs express the same idea through wildcard
positions such as `? super T` for consumed argument types and `? extends R` for
produced return types.

### 9. Annotation Targets

Open question:
- do we want annotations on global functions/method-like top-level `def` declarations as a first-class supported target
- do we want annotations on global variables/top-level bindings
- if top-level bindings become annotatable, immutable constants are the only viable target because top-level mutable bindings are not allowed
- whether annotated globals should affect module export/import metadata, runtime reflection, generated code, or only checker/tooling behavior

Leaning:
- global functions are probably useful annotation targets for routing, tests, effects, permissions, and generated bindings
- immutable top-level constants may be useful too, but annotation metadata should describe stable declarations, not changing state

### 10. Primitive Type Definitions

Primitive types such as `Int`, `Float`, `Str`, `Rune`, `Bool`, and `Unit` should eventually have their public companion/static-style signatures defined in Lume source instead of being scattered through checker, interpreter, and backend special cases.

Possible direction:
- define core/predef singleton-style surfaces such as `IntExt`, `FloatExt`, `StrExt`, `RuneExt`, and `BoolExt`
- keep the user-facing syntax as `Int.parse(...)`, `Float.parse(...)`, etc.
- lower or resolve those user-facing primitive companion calls to the corresponding Lume-defined implementation
- keep tiny native bridges only where the platform boundary requires it, such as parsing through Java exceptions

Goal:
- make primitive APIs discoverable in ordinary Lume files
- let typechecking, docs, and Java generation share the same signatures
- reduce ad-hoc hardcoded primitive method knowledge in compiler phases

## Longer-Term Ideas

### 11. Result / Either Style Error Values

Still open:
- whether `try`-style propagation should stay hardcoded to these builtins or later grow a broader protocol

Important design constraint:
- expressing "same container family, different success type" is hard without higher-kinded types
- a future generalized propagation protocol would need a way to express compatible container families and failure conversion without losing clarity

Clarification on "failure conversion":
- this does not necessarily mean superclass/subclass conversion
- the more likely model is wrapper-style conversion into a broader application error type
- example:
  - `readFile() Result[Str, IoError]`
  - enclosing function returns `Result[Int, AppError]`
  - failure conversion would mean allowing `IoError` to be turned into something like `AppError.Io(...)` during propagation

### 12. Smarter Type Narrowing

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

### 13. Reified Generic Follow-Ups

Open reified-generic follow-ups:
- whether expected return types should help infer a reified parameter when no ordinary argument carries it
- whether explicit multi-type-argument call syntax needs a dedicated parser form beyond `call[Type](...)`
- which additional library APIs should use `[reified A]` instead of explicit `Type[A]` values

Possible target-typed inference rule:

```txt
def load[reified A]() Option[A] {
    ...
}

user Option[User] = load()
```

This could infer `A = User` from the expected type and lower as if the call had
been written:

```txt
user Option[User] = load[User]()
```

The rule should stay conservative:
- infer reified type arguments from an expected assignment type, return type, or parameter type only when every reified parameter is pinned to a concrete runtime-denotable type
- do not infer from `Any`, `_`, wildcard captures, unresolved type parameters, or ambiguous overloads
- keep `value = load()` rejected when no expected type is available
- keep explicit `load[User]()` available when the expected type is not obvious enough

Parameter-based inference should also be considered:

```txt
def describe[reified A](value A) Str {
    typeOf[A].qualifiedName().orPanic()
}

name = describe(User { name: "Ada" })
```

In this shape, ordinary argument types can infer `A` and provide the hidden
`Type[A]` evidence. The same concrete-type restriction should apply: inference
from `Any`, `_`, captured unknowns, or an interface-typed local should reify the
static type only if that is exactly what the source type says. It should not
recover the hidden concrete implementation type behind an interface value.

Reifiable type arguments should probably include every closed type that has a
runtime descriptor:
- primitive types such as `Int`, `Float`, `Bool`, `Str`, `Rune`, and `Unit`
- classes, enums, enum payload cases through their enum type, singles, annotations, and named shapes
- interfaces, as interface metadata only
- tuples and function types, if their component types are also reifiable
- anonymous shapes only when the full static field shape is known
- generic instantiations such as `List[User]` only when every type argument is reifiable

The critical distinction:
- `Type[User]` means metadata for the concrete class `User`
- `Type[NamedShape]` means metadata for the shape descriptor
- `Type[SomeInterface]` means metadata for the interface itself, not the runtime implementer
- `Type[_]` should not be produced by reified inference, because `_` is an existential capture, not a runtime type name

Constraints to preserve:
- automatic reification of every generic parameter
- reified type parameters on classes, shapes, enums, annotations, or singles

### 14. Deferred Cleanup Follow-Ups

Open questions:
- whether runtime errors should also run pending defers
- whether future async/concurrency features need a stronger cleanup model

### 15. Flow Control Composition

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

### 16. Explicit Tuple Projection

Possible later syntax:

```txt
pair = tuple(instance)
```

Open question:
- whether an explicit `tuple(instance)` projection is useful enough to justify reintroducing positional views over named data
- if added, whether it should expose only visible fields and whether field order should be declaration order

## TBD

### Irrefutable and Refutable Binding

Open follow-ups:
- consider removing `orPanic` from `Option`, `Result`, and `Either`, or at least discouraging it, because `expect value <- source` is the clearer assertive extraction form
- if `orPanic` stays, treat it as a low-level/interoperability escape hatch rather than normal application style
- consider direct nested payload destructuring inside `if let`, for example `if let Some((_, initialY, _)) = rows.get(0) { ... }`
- for now, prefer `if let Some(row) = rows.get(0) { ... }` and destructure `row` on the next line inside the branch
- keep `if let` chaining limited to `&&` joins; if we ever extend it, do that deliberately rather than broadening it implicitly through general boolean syntax

Open `guard` surface question:
- current early-exit refutable binding starts directly with the pattern, for example `guard Some(item) = maybeValue else { ... }`
- consider requiring `guard let` instead, so the binding family is visually consistent:
  - `let PATTERN = value` for irrefutable binding
  - `if let PATTERN = value { ... }` for refutable branching
  - `guard let PATTERN = value else { ... }` for refutable early exit
  - `expect PATTERN = value` for assertive matching
- if `guard` keeps the current pattern-first form, consider adding boolean guards too:

```txt
guard ready else {
    return Err("not ready")
}
```

- without boolean guards, the keyword may read broader than the feature, because many readers expect `guard` to guard a condition as well as destructure a value

### Lambda Surface

Possible explicit lambda keyword direction:

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

Leaning:
- keep the keyword-free arrow form unless real readability problems show up
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

Leaning:
- `single` factory methods are useful as ordinary namespaced helpers, but hidden-field companion privileges should be a separate deliberate decision

## Suggested Priority Order

1. enum + pattern ergonomics
2. derived `Eq` / `Hashed`
3. stdlib collection/query growth
4. word equality operators
