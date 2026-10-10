# Syntax Reference

This file describes the language syntax that is available now.

## Built-In Data Types

Primitive types:

- `Int`
- `Float`
- `Bool`
- `Str`
- `Rune`
- `Unit`

Intrinsic collection types:

- `[K : V]` (map; nominal spelling `Map[K, V]` is also accepted)
- `Vector[T]` or `[T]`

`Vector` and `Map` are concrete classes with intrinsic bracket type/literal
syntax. The remaining collection classes are provided by the standard library.

Vector and map shorthand can be nested, for example:

- `[[Int]]`
- `[[[(Str, Int)]]]`
- `[Str : [Int]]`
- `[Str : [Int : Bool]]`

`T?` is shorthand for `Option[T]` in every type position:

```txt
count Int? = Some(5)
def find(id Int) User? = ...
values [Str?] = [Some("one"), None]
```

The shorthand may not be repeated. `Int??` is rejected because `??` is the
extract-or-fallback expression operator. Write `Option[Int?]` when a nested
optional type is intentional.

### Qualified Type Paths

A module imported without a symbol selector exposes its visible types through
the module alias in every type position:

```txt
use app/models

def display(user models.User) Str = user.name
def choose(users Vector[models.User]) models.User | models.Guest = users[0]

if value is models.User {
    println(value.name)
}
```

The grammar is `Identifier ("." Identifier)*`, followed by any generic type
arguments. Qualified paths therefore work in parameter and return types,
generic arguments, unions, bounds, annotations, and runtime type tests. The
first identifier must resolve to a used module alias and the final name must be
a visible type from that module. Selective aliases remain available when a
short local name is preferable:

```txt
use app/models/{User as ModelUser}

def display(user ModelUser) Str = user.name
```

### Type Aliases

`type` introduces a transparent name for supported non-declaration type
expressions such as primitives, named types, functions, collections, and
unions:

```txt
type UserId = Int
type Handler = fn(Request) Response
type Users = [User]
type Companion = Pet
```

An alias introduces no new nominal type. The alias and its recursively expanded
target are the same type for assignment, calls, inference, and code generation.
Aliases may refer to aliases declared before or after them; direct and indirect
cycles are rejected.

An anonymous shape schema cannot be a `type` alias target. Declare a reusable
record with `shape Name { ... }`, or keep the anonymous schema inline.

### Union Types

Use `|` to declare a value that may have any one of several types:

```txt
value Cat | Dog = Cat("Milo")

def describe(value Cat | Dog) Str = match value {
    case Cat { name } => "cat " + name
    case Dog { name } => "dog " + name
}
```

A union may be given an alias like any other type expression:

```txt
type Pet = Cat | Dog

pet Pet = Dog("Pip")
```

A union may also declare its alternatives inline. Each alternative states what
kind of value it is:

```txt
type Outcome =
    class Success { value Str }
    | shape Failure { message Str }
    | object Cancelled {}

ext Outcome {
    def result() Str = match this {
        case Success { value } => value
        case Failure { message } => "Failed: " + message
        case Cancelled => "Cancelled"
    }
}
```

Inline union rules:

- there is no `enum` declaration keyword; closed variants use declared unions
- a declared union contains at least two alternatives
- alternatives use `class`, `shape`, or `object`
- a class alternative has nominal class semantics
- a shape alternative has structural shape semantics with read-only field bindings
- an object alternative is fieldless and denotes one singleton value
- alternatives declare data only; shared behavior belongs in `ext UnionName`
- a declared union may list shared interfaces after its type parameters; the
  union's same-module extension methods may implement those contracts:

  ```txt
  type Option[T] with Iterable[T] =
      class Some { value T }
      | object None {}
  ```

- generic parameters belong to the union and are available to every alternative,
  for example `type Option[T] with Iterable[T] = class Some { value T } | object None {}`
- payload alternatives use their normal construction syntax, while object
  alternatives are referenced by name
- `match` over a declared union is exhaustive over its alternatives

`Pet`, `Cat | Dog`, and `Dog | Cat` are the same type. Union members are flattened
and de-duplicated, and their written order has no semantic effect.

Union assignment follows a subset rule. Every possible source alternative
must be assignable to at least one destination alternative:

```txt
source Cat | Dog = Cat("Milo")

same Dog | Cat = source             # valid: same alternatives
wider Bird | Dog | Cat = source     # valid: destination covers both
narrower Bird | Dog = source        # error: Cat is not covered
```

A concrete value is assignable to a union when it is assignable to at least
one member. A union is assignable to a non-union type only when every member is
assignable to that type. Narrow a union with `match` or `is` before using
members that are not shared by its alternatives. A union itself is not a
runtime type-test target.

Shape projection into a union must select one deterministic target. An exact
member type is kept without projection. Otherwise, an exact structural schema
is preferred; if there is no exact schema, exactly one wider-to-narrower shape
projection must be possible. Multiple exact schemas or multiple projection
targets are ambiguous and rejected. This rule is applied independently to
every concrete alternative of a source union, and union-member order never
breaks a tie.

```txt
shape XY {
    x Int
    y Int
}
shape XZ {
    x Int
    z Int
}
shape XYZ {
    x Int
    y Int
    z Int
}

original = XYZ(1, 2, 3)
selected XY | XZ = original                 # error: two projection targets
selected XY | XZ = XY { ...original }       # valid: target chosen explicitly
```

Common stdlib/prelude types:

- `Array[T]`
- `Set[T]`
- `LinkedList[T]`
- `Option[T]`
- `T?` (shorthand for `Option[T]`)
- `Result[T, E]`
- `Either[L, R]`
- `Iterable[T]`
- `Iterator[T]`
- `Type[T]`
- `TypeKind`
- `Ordered[T]`
- `Printer`
- `OS`

`Option[T]` implements `Iterable[T]`: `Some(value)` iterates once and `None`
iterates zero times. Consequently, iterable APIs can consume options directly.
For example, `Vector.flatMap[X](f fn(T) Iterable[X]) Vector[X]` accepts a
callback returning either a `Vector[X]`, an `Option[X]`, or another iterable.

## Any Type

`Any` is the top value type: any value can be assigned to `Any`, but `Any` is
not assignable back to a narrower type without an explicit safe form.

`Any(value)` is the explicit expression form of that widening:

```txt
number = Any(42)
text = Any("hello")
point = Any(Point(1, 2))
```

It accepts exactly one positional value and produces `Any`. It preserves the
operand's current static view and equality witness; it does not clone, convert,
or downcast the underlying value. A backend may box the value when its runtime
representation requires it. Applying it to a value that is already `Any` is
idempotent. Its lowering is identical to implicit assignment to `Any`.

These forms are invalid:

```txt
Any()                 # error: one value is required
Any(1, 2)             # error: only one value is accepted
Any { value: 1 }      # error: Any is not constructed with fields
```

Implicit widening remains the normal convenient form:

```txt
def printAnything(value Any) Unit = ...

value Any = "hello"
printAnything("hello")
printAnything(Any("hello")) # valid, but usually unnecessary
```

`Any(value)` is not a general cast. Constructing a narrower type from an `Any`
value does not downcast it:

```txt
user = User(anyValue) # constructor call, not a downcast

if let user User = anyValue {
    println(user.name)
}

user = match anyValue {
    case value User => value
    case _ => return Err(NotAUser)
}
```

Universal value operations:

- `value.toStr()` returns a `Str` rendering of the value
- `value.equals(other)` returns `Bool` and has the same statically typed equality semantics as `value == other`

`Any` does not support ordinary equality because it erases the static equality
domain. Narrow it before using `==`:

```txt
unknown Any = Point(1, 2)
point = Point(1, 2)

unknown == point # error

equal = match unknown {
    case other Point => other == point
    case _ => false
}
```

Strict equality adds a concrete-type restriction to ordinary equality:

```txt
interface Identified with Eq[Identified] {
    def id Int
}

left Identified = DatabaseRecord(7)
crossType Identified = CachedRecord(7)
sameType Identified = DatabaseRecord(7)
different Identified = DatabaseRecord(8)

left == crossType   # true: Eq[Identified] considers them equal
left === crossType  # false: different concrete classes
left === sameType   # true: same concrete class and Eq[Identified] says equal
left !== different  # true: same concrete class but unequal values
```

`===` means `sameConcreteType(left, right) && (left == right)`. `!==` is the
negation of that complete relation. Both operators use exactly the equality
contract selected for `==`; they do not select a concrete class's separate
contract or compare all stored fields.

For classes and declared-union payloads, concrete type identity is nominal. For
shapes it is the complete structural schema: field names and normalized field
types must match, while declaration name and field order do not matter. The
operands must otherwise satisfy the same static equality-domain rules as `==`.
In particular, interface operands require an `Eq[Interface]` contract visible
through the interface, and `Any` operands must be narrowed first.

Reference identity is explicit instance metadata rather than an equality
operator:

```txt
first = DatabaseRecord(7)
alias = first
separate = DatabaseRecord(7)

first.referenceId == alias.referenceId    # true
first.referenceId == separate.referenceId # false, even though `first === separate`
```

`referenceId` returns an opaque `ReferenceId`. Two such values compare equal if
and only if they identify the same instance. `ReferenceId` has built-in stable
equality and hashing, so it may be used in sets and as a map key. The token
retains a strong reference to its instance.

## Wildcard Capture

`_` inside a type argument is an existential capture, not a normal concrete type.
It means "some definite type, but this code does not know which one."

```txt
a Vector[_] = Vector(1, 2, 3)
b [_ : Str] = intStrMap()
c [_ : _] = strIntMap()
```

When a value is viewed through `_`, the unknown type is captured at that source:

```txt
first Any = a[0]      # allowed; Any accepts every captured value
captured = a[0]       # captured has the existential element type from a
sameCapture = captured
```

The captured type is only equal to itself when it comes from the same unknown
source:

```txt
def same[T](left T, right T) Unit {}

same(a[0], a[1])      # allowed; both values come from a
same(a[0], b[0])      # rejected if b is a different Vector[_] source
```

Capture is read-safe but not write-open. Methods that do not mention the
captured type can be called, while methods that consume it cannot accept an
arbitrary concrete value:

```txt
a.size                # allowed
a.add(7)              # rejected; add expects the captured element type, not Int

value Any = a[0]      # allowed
value SomeType = a[0] # rejected; captured element type is not SomeType
```

Tuple types:

- `(Int, Str)`

Singleton tuple types are not supported. `(Int,)` is invalid; use `Int`
directly.

Function types:

- `fn(Int) Str`
- `fn(Int, Bool) Unit`
- `fn() Unit`

Function types use `fn` with a parenthesized parameter-type list, followed by
the return type without an arrow. Whitespace between `fn` and `(` is
insignificant; `fn(Int) Int` is the canonical spelling, while `fn (Int) Int`
is also valid. Arrow forms such as `fn(Int) => Int`, `(Int) => Int`, and
`Int => Int` are invalid.
The return type may itself be a function type, as in
`fn() fn(Int) Str`. Lambda expressions remain keyword-free, for example
`value => value + 1`.

## Runtime Metadata

Runtime metadata is exposed through the `Type[A]` hierarchy declared in
`stdlib/runtime.lum`.

Use `typeOf[T]` to get metadata for a type:

```txt
userType Type[User] = typeOf[User]
```

Every value also has a synthetic `runtimeType` field:

```txt
user User = User { name: "Ada", age: 42 }
actual Type[User] = user.runtimeType
```

Reference-bearing values also expose a synthetic `referenceId` field:

```txt
first = User(7)
alias = first
separate = User(7)

id ReferenceId = first.referenceId
first.referenceId == alias.referenceId    # true
first.referenceId == separate.referenceId # false
```

`referenceId` is available on classes, objects, and concrete identity-bearing
collections. It is not available on primitives, tuples, shapes, declared-union
values, or unconstrained interface values. Narrow an interface value to a
reference-bearing concrete type before reading it.

Like `runtimeType`, `referenceId` is synthetic read-only metadata. It cannot be
declared, assigned, supplied to a constructor, reflected as a stored field, or
included implicitly by a spread. It participates in a shape only when included
under an explicit field label.

Runtime metadata types are generic over the represented type:

```txt
Type[A]   # exact typed metadata for A
Type[Any] # exact typed metadata specifically for Any
```

`typeOf[T]` returns `Type[T]`. `value.runtimeType` returns `Type[A]` for a
concrete statically known value type `A`. If the value is statically `Any` or
otherwise not known precisely, the represented type is captured and may be
written at use sites as `Type[_]`. This is still `Type[T]` with wildcard
capture, not a separate metadata type.

The current reflection ABI retains the compatibility names `EnumType`,
`EnumCase`, `TypeKind.Enum`, and `asEnum()` for closed declared-union metadata.
These are reflection API names only; source declarations use `type ... =` with
`class`, `shape`, and `object` alternatives.

Common metadata operations:

```txt
println(typeOf[User].name !)
println(typeOf[User].kind)

classType ClassType[User] = typeOf[User].asClass() !
fields = classType.fields
let Some { value as nameField } = classType.field("name") else panic("expected name field")
println(nameField.fieldType.name !)
println(nameField.isPrivate)

enumType EnumType[Status] = typeOf[Status].asEnum() !
let Some { value as pendingCase } = enumType.case("Pending") else panic("expected Pending case")
println(pendingCase.name)
constructedCase Result[Any, ReflectionError] = pendingCase.construct()
```

Safe reflective invocation uses `Result` values:

```txt
constructed Result[User, ReflectionError] = classType.construct("Ada", 42)

user User = constructed !
nameValue Result[Any, ReflectionError] = nameField.get(user)

let Some { value as greetMethod } = classType.method("greet") else panic("expected greet method")
greeting Result[Any, ReflectionError] = greetMethod.call(user)
```

Use postfix extraction when reflective failure should panic instead of remaining
in the value model:

```txt
greeting Any = greetMethod.call(user) !
```

Rules:

- `typeOf[T]` is a built-in type metadata operator, not an index operation
- `runtimeType` is available as a read-only synthetic field on values
- `TypeKind` includes `Class`, `Shape`, `Enum`, `Interface`, `Object`, `Annotation`, `Primitive`, `Tuple`, `Function`, and `AnonymousShape`
- field, method, parameter, and declared-union alternative metadata are runtime
  values; their read-only metadata uses getters such as `name`, `fieldType`,
  `isPrivate`, `params`, and `returnType`
- annotation lookup is typed and reified: use `metadata.hasAnnotation[Route]()` and `metadata.annotation[Route]()`
- reflective construction is supported for class and named shape metadata through `construct(args...)`
- reflective declared-union alternative construction is supported through the compatibility API `EnumCase.construct(args...)`
- annotations are metadata only; they cannot be constructed as runtime values in source code or through reflection
- reflective field reads use `Field.get(receiver)` and reflective safe method calls use `Method.call(receiver, args...)`

## Strings

String literals have interpreted and raw forms:

```txt
"hello"
"""hello
world"""
raw"hello"
raw"""hello
world"""
```

Interpreted strings support escapes and interpolation:

```txt
"hello $name"
"next ${count + 1}"
"money \$5"
"""hello $name
next ${count + 1}"""
```

Rules:

- `$name` interpolates a simple identifier expression
- `${...}` interpolates a full expression
- `\$` inserts a literal dollar sign
- `Str.size` returns the number of Unicode scalar values as `Int`; it does not
  count UTF-8 bytes or UTF-16 code units
- `Str.runeAt(index)` uses a zero-based Unicode-scalar index and returns `None`
  when `index` is negative or outside the string; use `runeAt(index) !` for
  assertive extraction
- `Str.isEmpty` reports whether the string contains no characters
- `Str.nonEmpty` reports whether the string contains at least one character
- `Str.trim()` removes leading and trailing whitespace
- `Str.trimLeft()` and `Str.trimRight()` remove whitespace from one side
- `Str.toLower()` and `Str.toUpper()` use Unicode case conversion and are not
  locale-sensitive
- `Str.contains(part)` performs literal substring membership; `contains` is the
  common membership spelling for strings and collections
- `Str.indexOf(part)` returns the zero-based Unicode-scalar index of the first
  literal match, or `-1` when no match exists
- `Str.split(separator)` treats `separator` as literal text and returns a
  growable `Vector[Str]`
- `Str.splitRegex(pattern)` explicitly interprets `pattern` as a regular
  expression and returns a growable `Vector[Str]`
- `Str.replaceFirstRegex(pattern, replacement)` and
  `Str.replaceAllRegex(pattern, replacement)` make regular-expression
  replacement explicit in the method name

Raw strings preserve their contents without escapes or interpolation:

```txt
raw"$name\n"
raw"""$name
\n"""
```

Multiline strings use triple quotes and preserve their line breaks. Use
`raw"""..."""` when `$` and `\` should remain literal.
Ordinary `"..."` and `raw"..."` strings cannot cross a physical newline; use
the `\n` escape for a line break inside an ordinary string or triple quotes for
multiline source text.

## OS / Printing

Console printing is available through `OS`:

```txt
OS.print("hello")
OS.println("hello")
OS.printf("value=%d\n", 42)
OS.stdout.println("hello")
OS.stderr.println("oops")
for argument <- OS.args {
    println(argument)
}
```

`OS.stdout` and `OS.stderr` implement `Printer`.

`OS.args` is a `Vector[Str]` snapshot of the program arguments. Pass arguments
to the interpreter after `--`; the command, source path, optional entry name,
and separator are not included:

```sh
lume run app.lum -- first "two words"
lume run app.lum alternateEntry -- first "two words"
```

Each `OS.args` access returns a fresh vector, so mutating it does not change the
process argument snapshot.

`Math.min` and `Math.max` select the smaller or larger of two values. Both
arguments must be the same numeric type; overloads are available for `Int` and
`Float`, and the result preserves that type:

```txt
smaller Int = Math.min(8, 3)
larger Float = Math.max(1.5, 2.25)
```

`panic(...)` and `assert(...)` are prelude functions, not `OS` methods:

```txt
panic("boom")
assert(ready)
assert(ready, "not ready")
```

## Use

Supported use forms:

```txt
use module/sub
use module/sub/*
use module/sub/A
use module/sub/A as B
use module/sub/{A, B as D, C}
```

Meaning:

- `use module/sub`
  qualified access through the module name, for example `sub.A`
- `use module/sub/*`
  use all visible symbols unqualified; extension methods declared by that module are also available as receiver methods in this file
- `use module/sub/A`
  use one symbol unqualified
- `use module/sub/A as B`
  use one symbol with a local alias
- `use module/sub/{A, B as D, C}`
  use a selected symbol set
- `use module/sub/SingletonName/*`
  use all visible singleton methods unqualified
- `use module/sub/SingletonName/{printLn as printN, print}`
  use selected visible singleton methods from a singleton

Built-in `OS` methods are available implicitly in every file, so `print(...)`, `println(...)`, and `printf(...)` work without writing `use OS/*`. Prelude functions like `panic(...)`, `assert(...)`, `ensure(...)`, and `identity(...)` are also available in every file. Fields like `OS.stdout` and `OS.stderr` still use explicit member access.

Extension methods are made available only by wildcard module use. They are visible in
the module where the `ext` block is declared and in files that write
`use module/sub/*`. Selective use forms such as `use module/sub/Name` do not
make extension methods available.

The standard spec helper module is brought in explicitly by test files. It is
not part of the prelude; specs are executed by the test runner:

```txt
use spec/*

class PrimitiveSpec with Spec {
    def it() Unit {
        5.shouldBe(5)
        "ok".shouldBe("ok")
    }
}
```

`spec` provides `Spec` and primitive `shouldBe` extension methods. A failed
`shouldBe` panics. `lume test file.lum` discovers every class or named object that
implements `Spec`, constructs it, and calls `it()`.

## Top-Level Declarations

Annotations are declared with `annotation`. They are shape-like metadata types:
- only visible immutable fields are allowed
- fields may have default values
- methods and custom constructors are not allowed

Examples:

```txt
annotation Route {
    path Str
    method Str = "GET"
}

type RouteVisibility =
    object External {}
    | object Internal {}

annotation Metadata {
    text Str
    code Int
    enabled Bool
    visibility RouteVisibility
    joinedPath Str
    total Int
    tags [Str]
    nested { name Str, value Int }
}

routePath Str = "/status"

object Routes {
    health Str = "/health"
}

@Route { path: routePath }
def status() Str = "ok"

@Route { path: "/health" }
def health() Str = "ok"

@Route { path: Routes.health }
def healthFromObject() Str = "ok"

@Route { path: "/health", method: "POST" }
def health2() Str = "ok"

@Metadata {
    text: "literal",
    code: 123,
    enabled: true,
    visibility: RouteVisibility.External,
    joinedPath: "/api" + "/health",
    total: 1 + 2,
    tags: ["a", "b"],
    nested: { name: Routes.health, value: 1 }
}
def richMetadata() Str = "ok"
```

`object Routes { ... }` declares one shared value named `Routes`, so `Routes.health`
is ordinary field access on that stable object value.

Annotation arguments are compile-time metadata values. They may only be literals, stable constants, aggregate literals made from allowed values, or constant expressions composed from allowed values:

- immutable top-level constants, including constants brought in with `use`
- immutable fields on named `object` values, such as `Routes.health`
- immutable constants through a module alias, such as `routes.healthPath`
- declared-union object alternatives, such as `RouteVisibility.External`
- arithmetic, comparison, boolean, and string-concatenation expressions whose operands are also annotation-safe

Calls, constructors, indexing, mutable object fields, ordinary instance field reads, `try`, `for ... yield`, `match`, `if`, lambdas, and blocks are rejected in annotation arguments. Top-level mutable bindings are not allowed at all, so they are rejected before annotation argument checking.

Supported annotation targets:

- top-level and nested `annotation`, `interface`, `class`, `shape`, and `object`
- declared unions introduced by `type Name = class ... | object ...`
- fields
- methods
- interface methods
- declared-union alternatives

Transparent aliases do not introduce runtime metadata and cannot be annotated:

```txt
@Serializable
type UserId = Int # invalid
```

Declared unions and their alternatives do introduce runtime metadata, so both
levels may carry annotations:

```txt
@Serializable
type Outcome =
    @Payload
    class Success { value Str }
    | @Payload object Cancelled {}
```

Module declaration:

```txt
module app
```

Top-level forms:

- `def`
- `annotation`
- `interface`
- `class`
- `shape`
- `object`
- `type`
- `ext TypeName`
- `name Type = expr`
- `private def`
- `internal def`
- `private name Type = expr`
- `internal name Type = expr`
- `private annotation`
- `internal annotation`
- `private interface`
- `internal interface`
- `private class`
- `internal class`
- `private shape`
- `internal shape`
- `private object`
- `internal object`
- `private type`
- `internal type`

Examples:

```txt
def greet(name Str) Str = "hello, " + name

interface Named {
    def label() Str
}

annotation Route {
    path Str
    method Str = "GET"
}

class Box[T] {
    value T
}

shape Point {
    x Int
    y Int
}

private shape InternalPoint {
    x Int
    y Int
}

object Counter {
    var count Int = 0
}

class Amount {
    value Int
    label Str
}

type OptionX[T] =
    class SomeX { value T }
    | object NoneX {}
```

### Nested Declarations

Named `class`, `shape`, `interface`, `object`, and `annotation` declarations,
as well as `type` aliases and declared unions, may appear inside any other
named declaration of those kinds:

```txt
class Parser {
    shape Position {
        line Int
        column Int
    }

    private class State {
        position Position
    }

    interface Input {
        def read() Str?
    }

    object Defaults {
        maximumDepth Int = 100
    }

    annotation Rule {
        name Str
    }

    type Offset = Int

    type Outcome =
        class Parsed { text Str }
        | object Empty {}
}
```

Inside the enclosing declaration, use the short name such as `Position`.
Outside it, use the qualified name such as `Parser.Position` or
`Parser.Outcome`. Normal visibility rules still apply. Visibility is
transitive through the declaration path: a public child of a private or
internal parent does not bypass that parent's visibility.

Nesting is lexical organization only. A nested declaration:

- is not an instance field and does not affect the enclosing constructor,
  storage, equality, or hashing
- does not capture an enclosing `this`, value, or generic parameter
- must declare and receive its own dependencies and generic parameters
- retains its ordinary semantics; in particular, a nested named `object` is one
  singleton rather than one value per enclosing instance

Named declarations, including `type` aliases and declared unions, are not
allowed in functions, methods, constructors, getters, lambdas, or nested
executable blocks:

```txt
def process() Unit {
    shape Entry { value Int } # invalid
}
```

Anonymous shape construction, anonymous `object` expressions, lambdas, and
local functions remain valid in callable bodies because they construct values
or declare callables without introducing a named type declaration.

Arbitrary statements such as `if`, `for`, `match`, `defer`, or expression statements are not valid at top level. Put executable code inside a function such as `def main() Unit { ... }`.

## Variable Declarations

Immutable local binding:

```txt
value = 1
name Str = "Ada"
```

Mutable local binding:

```txt
var count = 0
var total Int = 10
```

The compiler warns when a local binding is declared with `var` but is never
reassigned. Use an ordinary immutable binding in that case. Mutating an object
referenced by a binding does not reassign the binding itself:

```txt
values = [1]
values.add(2) # `values` does not need `var`
```

Top-level immutable bindings are also supported:

```txt
seed Int = 1
private internalSeed Int = 0
```

Top-level mutable bindings are not allowed. Mutable module state must live
inside a named `object`, class instance, or function local.

Fields without initializers are only valid in class-like field declarations:

```txt
class Box {
    private var cached Int
    private label Str
}
```

`private` and `internal` fields in classes and named objects may infer their
type from an initializer:

```txt
class Box {
    private count = 0
    internal var hits = 0
}

object Greeter {
    private hello = "Hello"
}
```

Visible class fields, shape fields, union-variant fields, and object fields still require explicit field types.

## Assignment and Update

Reassignment:

```txt
count := count + 1
```

`=` is for bindings and initialization, including field initialization inside a
constructor; `:=` is for statement-level reassignment.

Compound assignment:

```txt
count += 1
count -= 1
count *= 2
count /= 2
count %= 2
```

Compound assignments are mutation operators despite containing `=`. They
require an existing mutable binding or mutable field, and they never introduce a
new binding.

Constructor field initialization:

```txt
this.value = value
```

Inside `new`, direct writes to fields use `=` even for `var` fields or fields
that already have defaults. Constructor initialization should use `this.field`
because a bare `name = value` statement is a local binding. `:=` and compound
assignment are for post-construction mutation. Field reads and reassignments may
be bare when no local binding with the same name is in scope; use `this.field`
when a parameter/local shadows the field or when explicit receiver access reads
better.

Receiver field scope rules:

- Parameters may shadow receiver fields.
- Local bindings may not shadow parameters.
- Local bindings may not shadow receiver fields.
- Local bindings may not shadow another live local binding.
- Local bindings in disjoint scopes may reuse the same name.
- Unqualified field access is allowed only when no parameter/local with that name is in scope.
- `this.field` is always available inside instance methods and constructors.

Member reassignment:

```txt
count := count + 1
this.count := this.count + 1
```

Index assignment:

```txt
values[0] := 1
values[1] := values[0] + 4
```

Bracket access and assignment are unsafe operations supported by `Vector[T]`
and `Array[T]`; an invalid index panics. `LinkedList[T]` deliberately does not
support brackets because indexed traversal is linear.

Tuples support zero-based, read-only bracket access with a compile-time integer
literal. The compiler checks the bound and returns the exact element type:

```txt
pair (Int, Str) = (7, "seven")
number Int = pair[0]
text Str = pair[1]
```

A dynamic index such as `pair[index]`, a negative index, or an out-of-bounds
literal is rejected during compilation. Tuple indexed assignment is not
supported because tuples are immutable.

Vector slicing uses a half-open range: the start is included and the end is
excluded. Either bound may be omitted:

```txt
prefix = values[:5]      # values.slice(0, 5)
middle = values[1:5]     # values.slice(1, 5)
suffix = values[5:]      # values.slice(5)
copy = values[:]         # values.slice()
```

`slice()`, `slice(start)`, and `slice(start, end)` return a fresh shallow
`Vector` and evaluate the receiver once. Bounds must satisfy
`0 <= start <= end <= values.size`; an invalid range panics. Slice brackets
are available only on `Vector`; step syntax such as `values[1:5:2]` is not
supported. At expression start, `[]` remains the contextual empty vector/map
literal; `[:]` is meaningful only after a `Vector` expression.

Shape composition and exact update:

```txt
updated = value with {
    age: 42
    name: "Bob"
}

patch = { age: 43 }
updated2 = value with patch
```

Anonymous-shape spread:

```txt
copy = { ...value }

extended = {
    ...value
    location: "New York"
}

merged = {
    ...namePart
    ...agePart
}

selected = {
    ...point
    ...dot
    x: point.x
}

layered = {
    ...defaults
    override ...environment
    override ...commandLine
}
```

Spread entries copy fields from a class, shape, or anonymous-shape value into a
new anonymous shape. Ordinary spreads are collision-protected. The compiler
checks the complete literal, and every resulting field must have one
unambiguous final provider. A field has a final provider when it comes from only
one source, an explicit `field: value` selects it, or an
`override ...source` spread gives that source precedence over earlier spreads.

An explicit field resolves that field regardless of whether it appears before
or after the colliding spreads. Duplicate explicit fields remain invalid.
`override ...source` adds unique fields and selects that source for every field
that overlaps an earlier spread. A later ordinary spread can introduce a new
unresolved collision. This strict default prevents newly added source fields
from silently changing an existing merge.

For example, `{ ...point, ...dot }` is invalid when both values provide `x`.
Use `{ ...point, ...dot, x: point.x }` to resolve only `x`, or
`{ ...point, override ...dot }` to accept all current and future overlaps from
`dot`.

`base with patch` updates existing fields on a named or anonymous shape.
`patch` must also be a statically known named or anonymous shape. Every field in
`patch` must already exist on `base`, and each patch field type must be
assignable to the corresponding base field type. The result has the same shape
as `base`, and the source value is not mutated.

Classes do not support `with`. Copying a class implicitly would create unclear
object-identity, private-state, resource, and constructor-invariant semantics.
Classes that need copy-style updates should expose an explicit method:

```txt
updated = account.withBalance(42)
```

## Construction

This section defines target selection and brace interpretation for every
construction expression. Constructor declarations later in the reference
define which inputs a class accepts, but do not create a second expression
model.

Field braces construct values. Parsing is entirely syntactic:

```txt
{ field: value }  # construction
{ ...source }     # construction
{ value }         # block expression
{}                # empty construction; requires an expected target
```

The type checker then applies the construction fields to a unique concrete
class or shape target supplied by context. The shape target may be named or an
explicit anonymous schema. Without such a target, nonempty labeled fields or
spreads infer an anonymous shape:

```txt
point Point = {
    x: 10
    y: 20
}

user User = {
    name: "Ada"
    age: 42
}

anonymous = {
    x: 10
    y: 20
}

copy = { ...source }

nothing Unit = {}          # Unit, equivalent to ()
user EmptyUser = {}        # zero-argument contextual construction
names [Str] = {}           # zero-argument Vector construction

empty = {}                 # error: no construction target
empty = new {}             # error: no construction target
```

Empty braces use the expected type when one exists. A concrete class, named
shape, or collection target invokes its normal zero-argument construction and
must satisfy the same constructor/default rules as any other call. Expected
`Unit` produces `()`, and an explicit anonymous shape type supplies a structural
target. `Any`, an interface, a union, or no expected type does not identify a
construction target. An empty shape must therefore state its schema explicitly:

```txt
unknown Any = {}       # error: Any is not a concrete target
empty {} = {}          # explicit empty structural target
forced {} = new {}     # same target, explicit construction spelling
defaults {} = default {} # default construction of the same empty schema
point { x Int } = {}   # error: required field x is missing
```

An exact empty callable body has one deliberate default: it produces `Unit`.
This applies to direct bodies and unannotated `=` bodies:

```txt
def noop() {}
def noop2() = {}
```

An explicit return type remains an expected construction target, so these are
different:

```txt
def createCache() Cache = {} # construct Cache
def createCache() Cache {}   # error: empty Unit body does not return Cache
```

Context can flow from a typed binding, return type, indexed assignment, or a
single known function parameter:

```txt
def makePoint(x Int, y Int) Point = {
    x: x
    y: y
}

rollups[key] := {
    count: 0
    total: 0
}

save({ name: "Ada", age: 42 })
```

Expected constructor-input types also flow into nested construction. The inner
target therefore does not need to be written when its enclosing input
determines exactly one concrete class or named shape:

```txt
class Person {
    name Str
    age Int
}

class Team {
    leader Person
}

team Team = Team {
    leader: {
        name: "Ada"
        age: 10
    }
}
```

Write the inner target explicitly only when context does not determine it or
when overload resolution would otherwise have to choose a target.

Field punning is ambiguous inside bare braces because `{ x }` is a block.
Use `new { ... }` to force construction syntax. A punned field `x` means
`x: x` and is matched by name, never by position:

```txt
point Point = new { x, y }
user User = new { name, age }
anonymous = new { x, y }
empty {} = new {}
```

`new { ... }` uses a concrete class or shape target when context supplies
exactly one. A nonempty construction without a target infers an anonymous
shape from its fields. Empty construction still requires a target. An inferred
anonymous shape may then widen to `Any`, but it does not implicitly implement
an interface or pick one alternative of a union:

```txt
value Any = new { x, y }                 # anonymous shape widened to Any
value Printable = new { x, y }           # invalid
value Success | Failure = new { message } # invalid: no alternative is chosen
```

`new` may remain explicit even when labeled fields already make construction
unambiguous. Both forms are valid and have the same construction semantics; a
formatter may prefer the shorter first form:

```txt
point Point = { x: 1, y: 2 }
point Point = new { x: 1, y: 2 }
```

### Default initialization

`default { ... }` explicitly fills omitted immediate construction fields. A
declared field initializer is preserved. Otherwise, `Bool`, `Int`, `Float`,
`Str`, and `Rune` receive their primitive defaults, `Option[T]` receives
`None`, and another concrete field type is constructed only when its ordinary
accessible constructor accepts zero supplied arguments:

```txt
class RetrySettings {
    attempts Int = 3
}

shape Person {
    name Str = "Alex"
    family Str
    active Bool
    nickname Str?
    tags [Str]
    retries RetrySettings
}

person Person = default {}
# name == "Alex", family == "", active == false
# nickname == None, tags is empty, retries.attempts == 3

renamed Person = default { name: "Jordan" }
```

Default initialization is deliberately nonrecursive. It does not inspect a
nested type's fields and invent constructor arguments:

```txt
shape Point {
    x Int
    y Int
}

shape State {
    origin Point
    files [Str]
}

state State = default {} # invalid: Point requires construction inputs
state State = default { origin: Point(0, 0) } # valid
```

Empty collections qualify through their existing zero-argument constructors;
their element types do not need defaults. Explicit fields are evaluated once in
source order. Remaining automatic values are created once in declaration order.
`default { ... }` requires one concrete class, named shape, or anonymous-shape
schema from context; it does not select an interface implementation or a union
alternative.

Contextual construction never selects between overloaded concrete targets:

```txt
class Saver {
    def save(user User) Unit = ()
    def save(admin Admin) Unit = ()
}

saver.save(new { name: "Ada" }) # invalid: ambiguous target
saver.save(User { name: "Ada" }) # explicit and valid
```

The target can always be named explicitly. `Type { ... }` is named-field
construction and supports punning without `new`:

```txt
point = Point { x, y }
user = User { name, age }

point = Point { x }          # valid: punned identifier means `x: x`
point = Point { makeX() }    # invalid: arbitrary expressions need a field label
```

Qualification and explicit generic arguments do not change named-field
construction. Only the final path segment names the type; preceding segments
may be lowercase module aliases:

```txt
point = models.Point { x, y }
nested = Outer.Point { x, y }
box = Box[Int] { value }
qualifiedBox = models.Box[Int] { value }
aliased = ModelPoint { x, y }
```

Name resolution verifies that the complete target denotes a constructible type;
qualification and explicit generic arguments do not alter that decision.

Classifying leading braces selects the first expression; it does not terminate
the surrounding expression body. In callable bodies, lambdas, and match cases,
construction and ordinary block expressions continue through normal postfix and
infix parsing:

```txt
def readX() Int = { x: 1 }.x
def answer() Int = { 40 } + 2
def render() Str = { 42 }.toStr()

def choose(flag Bool) Int = match flag {
    case true => { 40 } + 2
    case false => 0
}
```

A bare block with no following operation remains a block body, and a direct
callable body without `=` remains unchanged:

```txt
def calculation() = { 40 }

def procedure() {
    println("done")
}
```

Contextual class construction applies only to the fresh brace construction
expression. Assigning an already-created structural value remains ordinary
assignment and never invokes a class constructor:

```txt
user User = { name: "Ada" } # valid: constructs User

data = { name: "Ada" }
user User = data             # invalid: a shape does not become a class
```

Parentheses are for positional construction and calls:

```txt
user User = User("Ada", 10)
maybe = Some(5)
```

`new(...)` may omit the type name only when context supplies exactly one class
or shape schema with a known positional input order:

```txt
worker Worker = new("Ada", 42)
point { x Int, y Int } = new(10, 20)
```

Unlike `new { ... }`, positional `new(...)` cannot invent field names and does
not infer an anonymous shape. A named shape uses declaration order; an explicit
anonymous shape uses the schema's written field order. Positional `new(...)` is
rejected without such a target and cannot target an interface, `Any`, union, or
unconstrained type parameter.

A class constructor target must therefore be uniquely determined, but it does
not have to be written at the construction site. `User { ... }` names it
explicitly; `user User = { ... }` obtains it from the expected type.

Braces are also the field construction form for declared-union payload alternatives:

```txt
maybe = Some { value: 5 }
```

Object alternatives are bare values, not calls:

```txt
none = None
```

`shape` is not an expression keyword. It remains available for named shape
declarations and shape alternatives inside declared unions. Expression-level
`shape { ... }`, `shape {}`, and
`shape with Interface { ... }` are invalid. Use field braces, `new { ... }`,
and `object with Interface { ... }` respectively.

Construction can project a wider shape by visible field name. This includes
expected types flowing through generic callbacks such as `Option.map`. Given
`StoredRollup` with all `Rollup` fields plus additional fields, these forms all
construct the same narrower `Rollup` value:

```txt
def named() Rollup? = source().map(r => Rollup { ...r })
def fieldBraces() Rollup? = source().map(r => { ...r })
def contextualNew() Rollup? = source().map(r => new { ...r })
```

`Rollup { ...r }` names the target directly. Bare field braces and contextual
`new` obtain `Rollup` from the callback's expected return type. In each case,
required target fields must exist with assignable types; extra source fields
are discarded rather than copied into the narrower result.

Assignment from a wider shape materializes a new value of the target shape. It
copies only the target fields by name and permanently discards extra fields:

```txt
stored = StoredRollup { total: 7, label: "week", internalId: 99 }
rollup Rollup = stored

rollup.total       # valid
rollup.internalId  # error: not present in Rollup
rollup.runtimeType # Rollup
```

Projection is a shallow copy. Field bindings are copied into the new shape;
nested classes and mutable collections retain their existing references. The
source value itself is unchanged.

The same projection rule applies to parameter passing, returns, casts, and
collection storage. The projected value has no hidden source shape and cannot
later be narrowed back to it:

```txt
point Point = Point3D(1, 2, 3)
point.z # error: Point does not expose z

match point {
    case Point3D { z } => println(z) # does not match
    case _ => println(point.runtimeType.name !) # Point
}
```

Spreading a projected value sees only the fields retained by that value.
Every explicit construction entry must name an accepted field or constructor
input, so likely misspellings are rejected:

```txt
larger = { x: 1, y: 2, yy: 3 }
point Point = { ...larger }       # valid: `yy` is projected away

point Point = {
    x: 1
    y: 2
    yy: 3                         # invalid: no `yy` construction input
}
```

A member expression in a construction entry must have an explicit field label:

```txt
def selected() Rollup? = source().map(r => Rollup {
    total: r.total
    label: r.label
})
```

Only bare identifiers support field punning. Forms such as
`Rollup { r.total }` and `new { r.total }` are invalid;
write `total: r.total`. This keeps the constructed field name explicit when the
value comes from member access. Other unlabeled expressions also remain invalid
inside construction braces.

Anonymous behavior uses `object with`:

```txt
value = object with Printable {
    x Int = 10

    def print() Unit = println(x)
}
```

`object { ... }` and `object with Interface { ... }` synthesize anonymous
nominal object implementations. These forms determine their own concrete type;
they are not constructor calls.

Anonymous shape fields may infer their type from the initializer:

```txt
a = 1
b = {
    count: a
}
```

Fields in construction braces may be separated by commas, newlines, or both:

```txt
user = { name: "Ada",
    age: 10
}
```

Anonymous shape type:

```txt
def describe(user { name Str, age Int }) Str =
    user.name + " is " + user.age

def project(user { name Str, age Int }) { name Str } =
    { name: user.name }
```

Anonymous shape types in parameters, return types, fields, and local bindings
use bare braces. The `shape` prefix is not valid in those inline type positions.

Anonymous shape schemas cannot be named with `type`:

```txt
type Result = { x Str } # invalid
```

Use a named shape declaration when the schema needs a reusable name.

A named shape uses a shape declaration:

```txt
shape Session {
    start Int
    end Int

    def duration() Int = this.end - this.start
}

session = Session(10, 20)
```

Every named declaration kind has one declaration form:

```txt
class A { ... }
interface B { ... }
shape C { ... }
object D { ... }
annotation E { ... }
```

Declaration-valued aliases are not supported. For example, these are invalid:

```txt
type A = class { ... }
type B = interface { ... }
type C = shape { ... }
type D = object { ... }
type E = annotation { ... }
```

Use `type` for transparent aliases such as `type UserId = Int`,
`type Users = [User]`, and `type Handler = fn(Request) Response`.

Declaration keywords may follow `=` only as named alternatives of a declared
union, for example:

```txt
type Outcome =
    class Success { value Str }
    | shape Failure { message Str }
    | object Cancelled {}
```

A declared union requires at least two alternatives. Each alternative supplies
its own name; the union name does not stand in for an omitted alternative name.

`shape` is not a construction expression. Anonymous structural values use
labeled/spread field braces or forced `new { ... }`. Positional construction
requires an existing concrete target, written as `Point(...)` or supplied by a
unique expected type to `new(...)`.

Tuples do not construct shapes or classes. Class construction follows the
target-selection rules in [Construction](#construction): its target must be uniquely
determined, but does not have to be written at the construction site.

```txt
user User = { name: "Ada", age: 10 }
person Person = Person("Ben", 12, "NYC")
profile MixedProfile = {
    name: "Liam"
    age: 8
}
tail HiddenTail = new("Ada", 4)
settings Settings = new {}
```

Named shapes are data-only structural field views:
- fields are always visible and read-only
- fields are declared in the `shape` body
- methods are declared directly in the `shape` body after fields
- custom `new` constructors are not allowed
- shapes may declare interface bounds with `shape Name with Interface`
- brace field construction uses `ShapeName { field: value }`
- positional construction uses `ShapeName(...)`

Shapes are shallowly immutable: their field bindings cannot be replaced, but a
value reached through a field keeps its own mutability rules. Read-only fields
do not imply deeply immutable values or pure methods:

```txt
shape Batch {
    items [Int]
}

batch Batch = Batch([1])
otherItems = [2]

batch.items := otherItems # error: `items` is a read-only field
batch.items.add(42)        # valid: Vector remains mutable
```

```txt
shape Point {
    x Int
    y Int
    def sum() Int = this.x + this.y
}

interface Named {
    def label() Str
}

shape NamedPoint with Named {
    x Int
    y Int
    def label() Str = this.x + "," + this.y
}

origin = Point(0, 0)
named = Point { x: 3, y: 4 }
positional Point = Point(5, 6)
```

Class and shape call sites use the authoritative expression model in
[Construction](#construction). Runtime-backed collections such as `Vector`, `Map`, `Array`,
`LinkedList`, and `Set` use ordinary class construction; `Range(...)` is a
stdlib factory. The rules below define class constructor contracts rather than
a separate construction-expression model.

```txt
class Holder {
    payload { x Int }
}

payload = { x: 7 }
fromValue = Holder(payload)     # one positional shape argument
fromLiteral = Holder({ x: 7 }) # one positional shape argument
named = Holder { payload }      # one named input using field punning
```

Explicit constructor rules:

- `new(field Type = default, other Type) { ... }` declares explicit constructor inputs; defaults may appear anywhere
- constructor parameters do not have to be class fields; they are inputs to the constructor body
- `Type { field: value, other: value }` matches explicit constructor inputs by parameter name
- `Type(value, otherValue)` fills the same explicit constructor inputs by declaration order
- named construction may omit any constructor parameter that has a default
- positional construction fills a prefix of constructor parameters in declaration order
- a positional call may stop only when every remaining constructor parameter has a default
- positional arguments never skip an earlier default to initialize a later parameter
- if any explicit `new` exists, implicit field construction is disabled for that class
- explicit constructors may use one trailing variadic constructor parameter such as `items [T] vararg`
- a variadic constructor parameter receives the extra positional arguments as `[T]`
- only one variadic constructor parameter is allowed
- construction fields can target a variadic constructor parameter by passing a `[T]` value
- variadic constructor parameters may have a default `[T]` value

```txt
class Article {
    body Str
    title Str
    new(body Str = "body", title Str) {
        this.body = body
        this.title = title
    }
}

full Article = Article("custom body", "Intro")
named Article = Article { title: "Intro" }
custom Article = Article { body: "custom body", title: "Intro" }

# Invalid: the argument initializes body, leaving title unset.
# Article("Intro")
```

Implicit field construction rules:

- if a class has no explicit `new`, the compiler synthesizes one stable constructor contract from its public fields
- the implicit constructor contract is the same in every module and from every call site
- public fields without initializers are required constructor inputs
- public fields with initializers are optional constructor inputs
- initialized public fields may appear anywhere in the declaration
- `private` and `internal` fields never become implicit constructor inputs
- every non-public field must have an initializer when a class relies on implicit construction; otherwise the class must declare `new(...)`
- construction braces check the synthesized public-field shape
- `Type {}` works when the public constructor contract has no required inputs
- positional construction follows declared public-field order
- a positional call may stop only when every remaining public field has an initializer
- positional arguments never skip an initialized field to initialize a later field
- the declaration position of initialized non-public fields does not affect positional construction
- whether a class field is mutable or read-only does not affect explicit spread construction
- named class values do not structurally convert to other named class values

```txt
class Account {
    owner Str
    internal region Str = "US"
    balance Int
    private cache Cache = Cache()
}

# The same public contract is used inside and outside this module.
account = Account("Ada", 100)

class SecuredAccount {
    owner Str
    private token Str
}

# Invalid without an explicit `new(...)`: `token` has no initializer.
```

## Brace Disambiguation

Braces carry several meanings. The parser chooses by the tokens before and inside the braces:

```txt
{ field: value }                 # contextual construction or anonymous shape construction
{ ...source }                    # anonymous or contextual shape/class construction
{ expr }                         # block expression
{}                               # empty contextual construction; target required
Type { field: value }            # brace field construction or union payload
Type { field }                   # brace construction with a punned field
call { x => ... }                # trailing lambda
object { field Type = value; def method() Type = value } # anonymous object
object with Interface, Other { field Type = value; def method() Type = value } # anonymous object implementing interfaces
new(field Type)                  # constructor declaration
new(value, other)                # contextual positional construction
new { field: value }             # forced named-field construction
new { field }                    # forced punned-field construction
new {}                           # forced empty construction; target required
```

Single-expression braces such as `{ value }` are block expressions, not
anonymous shapes. Use `new { value }` when `value` is a punned field. Bare `{}`
and `new {}` are empty construction and both require an expected target.

Braces that a declaration or control-flow construct requires remain body
delimiters rather than expressions:

```txt
class Marker {}                  # empty declaration body
def noop() Unit {}               # empty callable body
def noop2() = {}                 # inferred empty callable body; Unit
def noopValue() Unit = {}        # expected Unit produces ()
def make() EmptyUser = {}        # zero-argument construction expression
```

Brace classification uses the first syntactic entry, not punctuation found
later in the body. A leading construction field or spread selects an anonymous
shape; otherwise bare braces select a block. When braces follow a callee, an
explicit lambda head is recognized before shape construction. Commas in an
ordinary block statement therefore do not reclassify the block:

```txt
result = {
    left, right = 1, 2
    left + right
}

consume { value =>
    left, right = 1, 2
    value + left + right
}
```

The same classification applies anywhere a brace-delimited expression body is
accepted: after a callable-body `=`, a lambda `=>`, a match-case `=>`, and
`yield`. Labeled fields and spreads are construction expressions in every one
of those positions:

```txt
def origin() Point = { x: 0, y: 0 }

mapper = value => { x: value, y: 0 }

point Point = match ready {
    case true => { x: 1, y: 2 }
    case false => { ...fallback }
}

points = for value <- values yield {
    x: value
    y: 0
}
```

Braces belonging to a declaration or control-flow construct remain that
construct's body. Ambiguous punning still requires `new { x, y }`.

Shape conversion rules:
- field names and field types must match at compile time
- extra fields are allowed when passing a value to a narrower shape
- missing fields are rejected
- defaults are not part of the shape syntax
- shape-to-shape assignment is structural by field names and field types
- class-to-shape assignment is not implicit; construct the target shape explicitly with `{ ...instance }`
- shape-to-interface follows the shape's explicit `with Interface` bounds
- class-to-interface-through-shape is not automatic; explicitly construct the interface-bearing shape first
- only visible class fields may be used by explicit shape construction
- an already-created shape value does not implicitly become a class; use a class constructor such as `Pixel { ...point }`
- tuple-to-shape and tuple-to-class are not allowed; use named shape construction, class constructors, or anonymous construction fields
- ordinary calls may still accept named anonymous shapes in parentheses, for example `describe({ name: "Cara", age: 14 })`
- construction fields inside braces use `field: value`; bare `field` is shorthand for `field: field`
- bare punned fields require `Type { field }` or `new { field }`; `{ field }` is a block
- construction fields cannot include a type; put an anonymous shape type on a
  binding, field, parameter, or return declaration, and use a named `shape`
  declaration when the schema needs a reusable name
- single-expression braces like `{ value }` are still block expressions, not anonymous shapes

Shape equality is structural across shape declarations:

- comparing two shapes with `==`, `!=`, or `equals(...)` requires the same complete set of field names and normalized field types
- field declaration order may differ
- the right operand is converted to the left operand's shape by field name, then ordinary value equality is applied
- unlike shape assignment, equality does not ignore extra fields
- width-compatible shapes must first be projected through a narrower typed binding, parameter, return, cast, or collection element type
- generated Java shape records define field-based `equals` and matching `hashCode` methods; hash inputs are ordered by field name so declaration order does not change the hash

```txt
shape Point { x Int, y Int }
shape Position { y Int, x Int }
shape Point3D { x Int, y Int, z Int }

Point(1, 2) == Position(2, 1) # true: fields match by name
Point(1, 2) == Point3D(1, 2, 3) # error: schemas differ

point2d Point = Point3D(1, 2, 3)
point2d.runtimeType.name ! # Point
point2d == Point(1, 2)     # true
point2d === Point(1, 2)    # true: both values now have the Point schema
point2d is Point3D         # false: z was discarded
```

Other equality domains are nominal. The operands must have the same normalized
type. Classes may use equality only when they explicitly implement
`Eq[ClassName]`; interface values require a compatible explicit `Eq` contract.
Different classes are not directly comparable. Values widened to `Any` must be
narrowed before comparison.

| Operands | `==` / `!=` |
| --- | --- |
| same shape schema | field equality |
| different shape names, same schema | field equality by name |
| width-compatible shape schemas | error; project to a common narrower shape first |
| class and shape | error; project explicitly before erasure |
| same class with `Eq[Class]` | declared class equality |
| different classes | error |
| `Any` and a typed value | error; narrow first |
| `Any` and `Any` | error; narrow first |
| interface values | requires an explicit compatible `Eq` domain |

Every equality path uses the same contract recursively. Shape fields, tuple
items, union payloads, `Set` membership and deduplication, and `Map` keys invoke
the participating type's declared equality. Collections never replace a
class's `equals` method with direct comparison of its stored fields.

`Hashed[T]` extends `Eq[T]` and declares `hash() Int`. Equal values must return
the same hash. Every shape derives `Eq` structurally and derives `Hashed[Shape]`
only when every field type is hashable:

```txt
interface Eq[T] {
    def equals(other T) Bool
}

interface Hashed[T] with Eq[T] {
    def hash() Int
}

class Map[K with Hashed[K], V] {
    # ...
}
```

- primitives and singleton object values are intrinsically hashable
- a tuple derives `Eq` when every item type satisfies `Eq`, and derives `Hashed` when every item type satisfies `Hashed`
- a declared union derives `Hashed` only when every shared field and every payload field of every alternative is hashable; zero-payload object alternatives are always safe
- nested shapes are hashable when their own fields are recursively hashable
- a class is hashable only when it explicitly implements `Hashed[ClassName]`, including `equals` and `hash`
- a type parameter is hashable only when it has a `Hashed[T]` bound
- interfaces, functions, `Any`, and arbitrary classes do not implicitly satisfy `Hashed`

Hash derivation is separate from shallow field immutability. It establishes
that each field type supplies compatible equality and hashing operations; it
does not freeze reachable values or guarantee that user-defined mutable hashed
objects retain the same hash after mutation.

```txt
class StableId with Hashed[StableId] {
    value Int

    def equals(other StableId) Bool = this.value == other.value
    def hash() Int = this.value
}

shape CacheKey {
    id StableId
    version Int
}

def cache[T with Hashed[T]](key T) Unit = ()

cache(CacheKey(StableId(1), 2))

pair = (StableId(1), 2)
cache(pair)

type LookupKey =
    class Named { value Str }
    | object Default {}

keys [LookupKey: Int] = [LookupKey.Named("primary"): 1]
```

`Map[K, V]` (normally written `[K: V]`) requires `K` to satisfy `Hashed[K]`.
This makes semantic equality and compatible hashing part of the key type's
public contract, regardless of the storage strategy used by a particular
runtime.

```txt
shape Point {
    x Int
    label Str
}

shape ReorderedPoint {
    label Str
    x Int
}

same = Point(1, "one") == ReorderedPoint("one", 1) # true
```

Examples:

```txt
shape Point {
    x Int
    y Int
}

class Pixel {
    x Int
    y Int
}

point Point = Point(1, 2)              # explicit named-shape positional construction
contextual Point = new(1, 2)           # contextual named-shape positional construction
anon = { x: 1, y: 2 }                  # anonymous structural construction
pixel = Pixel { x: 1, y: 2 }           # explicit structural construciton
fromClass Point = { ...pixel }         # explicit class -> shape snapshot
backToClass Pixel = Pixel { ...fromClass } # normal class construction
named Point = { x: 1, y: 2 }           # contextual named-shape construction

user User = ("Ada", 10)                # invalid: tuple -> class
point Point = (1, 2)                   # invalid: tuple -> named shape
anon { x Int, y Int } = new(1, 2)      # x = 1, y = 2 from schema order
reversed { y Int, x Int } = new(1, 2)  # y = 1, x = 2
unknown = new(1, 2)                    # invalid: no concrete expected target
user User = { name: "Ada", age: 10 }   # contextual class construction
empty = {}                             # invalid: no construction target
explicitEmpty = new {}                 # invalid: no construction target
emptyShape {} = {}                     # explicit empty anonymous shape
forcedEmpty {} = new {}                # same explicit target
defaultEmpty {} = default {}           # same target with default initialization
nothing Unit = {}                      # Unit from expected type
```

Anonymous shape field types come from the surrounding declaration:

```txt
user { name Str, age Int } = {
    name: "Ada"
    age: 42
}
```

Explicitly labeled construction fields use `field: value`; a bare identifier
uses field punning and means `field: field`. The combined `field Type: value`
form is invalid. Put the anonymous shape type on the binding, parameter, or
return value, or introduce a named `shape` declaration.

## Functions and Methods

`def` is required for top-level functions, local functions, and methods.
Constructors are the exception: they begin with `new` and never use `def`.

```txt
def greet(name Str) Str = "hello, " + name
```

Function-valued bindings carry the explicit `fn` type marker, as in
`mapper fn(Int) Int`. Whitespace between `fn` and the parameter list is
insignificant, so `fn (Int) Int` is equivalent; `fn(Int) Int` is the canonical
spelling.

Expression-bodied function:

```txt
def greet(name Str) Str = "hello, " + name
```

Block-bodied function:

```txt
def add(left Int, right Int) Int {
    return left + right
}

def addWithEquals(left Int, right Int) Int = {
    return left + right
}
```

Callable block bodies may include `=` or omit it. Expression-bodied callables
still use `=`.

When the return type is omitted, the body introducer determines the return
rule:

```txt
def reset() {
    this.value := 0
}                         # implicit Unit

def isMissing(value Int?) = {
    value is None
}                         # inferred Bool
```

A direct `{ ... }` body without a return type is a `Unit` callable, so
returning a value from it is an error. An `= expression` or `= { ... }` body
without a return type infers its result type from the body. An explicit return
type always takes precedence.

### Getters

A method declared without an adjacent `()` parameter list is a getter. In
callable declarations, whitespace before `(` is meaningful: an adjacent `(`
starts method parameters, while a separated parenthesized type is the getter's
return type:

```txt
def coordinates() (Int, Int) = (x, y)  # method returning a tuple
def coordinates (Int, Int) = (x, y)    # getter returning a tuple

def values() [Int] = items             # method returning a vector
def values [Int] = items               # getter returning a vector

def callback() fn() Int = action       # method returning a function
def callback fn() Int = action         # getter returning a function
```

Getters use the ordinary type grammar, including bracket collection shorthand
and anonymous shape types:

```txt
def counts [Str: Int] = counters
def rows [[Int]] = nestedRows
def optionalValues [Int?] = values
def callbacks [fn(Int) Int] = handlers

interface Positioned {
    def position { x Int, y Int }
}
```

Whitespace before a bracket also carries the normal declaration distinction. A
bracket touching the callable name begins a generic clause; a separated bracket
begins a getter return type:

```txt
def map[T](value T) T = value          # generic method
def values [Int] = items               # getter returning Vector[Int]
```

Function and method parameter lists must touch the callable name, or the final
`]` of a generic clause:

```txt
def calculate(input Int) Int = input
def convert[T](input T) T = input

def calculate (input Int) Int = input  # error: remove the space before `(`
def convert[T] (input T) T = input     # error: remove the space before `(`
```

Getters are read through ordinary member access and still lower to
zero-argument methods at runtime:

```txt
class Account {
    name Str
    balance Int

    def displayName Str = this.name

    def isPositive Bool {
        current = this.balance
        current > 0
    }
}

account Account = Account("Cash", 25)
println(account.displayName, account.isPositive)
```

Inside the declaring type, a getter may also be read through the implicit
receiver:

```txt
def summary Str = displayName + ": " + this.balance.toStr()
```

Getter rules:

- getters require an explicit return type
- getters have no parameter list and are accessed without parentheses
- getters cannot declare type parameters or generic conditions
- getters may declare local bindings
- direct mutation of class or shape fields is rejected, including indexed
  mutation rooted in a field
- a non-function getter value cannot be called; `value.getter()` is valid only
  when `getter` returns a function, in which case member access reads the getter
  and `()` invokes that returned function

Getters may be declared by classes, shapes, interfaces, objects, and extension
blocks. The current compiler enforces direct read-only field access. Proving
that a getter is transitively read-only requires following aliases, called
methods, callbacks, interface dispatch, and foreign calls. That is a separate
effect-analysis subsystem, not a local scan for assignment statements. Effect
information can remain compiler-internal initially; user-facing effect syntax
is a separate design decision.

An argument list after getter access never invokes the getter itself. It calls
the value returned by the getter, so function-valued getters compose normally:

```txt
class Factory {
    def creator fn() Int = () => 42
}

answer = Factory().creator() # read `creator`, then call the returned function
```

Generic function:

```txt
def id[T](value T) T = value
```

Generic clauses:

```txt
def identity[T](value T) T = value

def invoke[T with Callable](value T) Unit =
    value.call()

class Merger[L, R] {
    def merge[when L = R]() L =
        ...
}

class Context[GlobalT, GlobalR] {
    def something[
        LocalT with Callable,
        LocalR
        when GlobalT with Callable,
             LocalR = GlobalR
    ](value LocalT) GlobalT =
        ...
}
```

A direct bound stays attached to a type parameter declared by that clause:
`T with Callable`. Conditions involving multiple parameters or an enclosing
type parameter follow one `when`, inside the same brackets.

Rules:

- ordinary local type parameters use `T`
- a local parameter may carry a direct interface bound: `T with Callable`
- `when` introduces broader bound and exact-type equality conditions
- a clause contains at most one `when`; separate its conditions with commas
- write `[when L with Callable, L = R]`, not multiple `when` keywords
- a bound condition's left side must be a local or enclosing type parameter
- bounds must name interfaces
- `L = R` requires both sides to resolve to exactly the same type
- conditions are checked after explicit type arguments and argument inference

Generic type declarations use the same direct-bound and `when` rules. A use of
the resulting type must satisfy its conditions:

```txt
class Box[T with Callable] {
    value T
}

box Box[Action] = Box { value: Action {} }
```

The core `Either[L, R]` uses an owner equality condition for `merge`:

```txt
merge[when L = R]() L

left Either[Str, Str] = Left("problem")
value Str = left.merge()
```

`merge` is unavailable when the left and right types differ.

Reified generic functions and methods:

```txt
def typeName[reified A](value A) Str =
    typeOf[A].name !

def metadata[reified A]() Type[A] =
    typeOf[A]

name = typeName(User { name: "Ada" }) # A inferred from value
userType = metadata[User]()           # explicit because no value carries A
```

`reified A` means the callable receives hidden runtime type evidence for `A`.
Inside that callable, `typeOf[A]` is valid and returns the caller's concrete
`Type[A]`.

Rules:

- `reified` is allowed only on function and method type parameters.
- Generic type parameters are not reified by default.
- `typeOf[A]` is rejected inside `f[A]` unless `A` is marked `reified` or the function accepts an explicit `Type[A]` value.
- If `A` appears in ordinary arguments, the call may infer it: `typeName(user)`.
- If no argument determines `A`, pass it explicitly: `metadata[User]()`.
- Type declarations cannot use `reified`: `class Box[reified A]` is invalid.

Bracket application is resolved from the expression before the brackets, not
from the presence of a following call. A generic function, method, or type uses
the brackets as explicit type arguments. An indexable value uses them as an
index or key expression. Parentheses then invoke whichever callable value that
operation produces:

```txt
metadata[User]()       # explicit generic call
entries["a"]           # indexing
handlers[key]()        # index, then call the selected function value
service.handlers[key]() # read a getter, index its result, then call it
```

The receiver and the bracket contents are resolved semantically. A type name is
a type argument only when the receiver supports generic application; a value
such as `typeOf[User]` remains an ordinary index expression when the receiver is
indexable. Getter-local type parameters are not supported, so a non-generic
getter returning a collection is always read before its result is indexed.

Generic types follow the same explicit-or-inferred construction rule. A
constructor invocation may provide every type argument, or omit the complete
type-argument list and let Lume infer it from constructor arguments and the
expected type:

```txt
users Vector[User] = Vector()          # User comes from the expected type
lookup Map[Str, User] = Map()          # both arguments come from the expected type
result Result[Int, Error] = Ok(42)     # remaining context determines Error
box Box[Str] = Box("hello")            # Str agrees with the argument and context

explicitSet = Set[Str]()
explicitMap = Map[Str, User]()
inferredBox = Box("hello")
contextualBox Box[Str] = new("hello")  # new infers the complete target type
contextualSet Set[Str] = new()
```

Construction requires all generic arguments or none. Partial application and
existential placeholders are not constructor inference syntax:

```txt
Map[Str]()     # invalid: Map requires both K and V
Set[_]()       # invalid: omit [..] to infer, or provide a concrete type
Set[]()        # invalid: an explicit type-argument list cannot be empty
value = Set()  # invalid: neither arguments nor an expected type determine T
```

An empty generic constructor therefore needs either explicit arguments or an
expected type. Lume does not infer `Any` as a fallback:

```txt
names Set[Str] = Set()
names = Set[Str]()
anything Set[Any] = Set()
```

Function and method parameters may end with one variadic vector parameter. `vararg`
is written after the parameter type:

```txt
println(value [Str] vararg) Unit
printf(format Str, value [Str] vararg) Unit
```

The parameter is available as `[T]` inside the body, and call sites pass the
extra values positionally.

```txt
callable-parameter    = name Type [vararg] [= default]
                      | name => Type [= default]
constructor-parameter = name Type [vararg] [= default]
```

Rules:

- `vararg` is postfix-only; the removed prefix form `vararg values [T]` is invalid.
- A variadic parameter must have an explicit vector type `[T]`.
- Only the final parameter may be variadic.
- A parameter list may contain at most one variadic parameter.
- A by-name parameter cannot also be variadic.
- Defaults may appear anywhere in a parameter list.
- Positional arguments bind a contiguous declaration-order prefix.
- A positional call may stop only when every remaining parameter has a default.
- Named calls may omit defaulted parameters anywhere.
- Positional arguments never skip a parameter.
- A variadic parameter may have a default vector value, written after `vararg`.

The minimum positional arity is the position of the last required parameter.
If every parameter has a default, the minimum is zero:

| Declaration | Valid positional arities |
| --- | --- |
| `(default, required)` | 2 |
| `(required, default)` | 1, 2 |
| `(required, default, required)` | 3 |
| `(default, default)` | 0, 1, 2 |

```txt
def connect(protocol Str = "https", host Str, port Int = 443) Connection = ...

connect("https", "example.com")
connect("https", "example.com", 8443)
connect(host = "example.com")
connect(host = "example.com", port = 8443)

# Invalid: the argument initializes protocol, leaving host unset.
# connect("example.com")
```

```txt
new(segments [Str] vararg = ["tmp"]) {
    this.segments = segments
}
```

Call sites may spread an existing vector into a variadic tail with `...`:

```txt
extra = ["beta", "gamma"]

describe("task", "alpha", ...extra, "omega")
```

Spread arguments are valid only as positional arguments for a `vararg`
parameter. Fixed-arity parameters reject `...value`.

Function and method parameters may be by-name:

```txt
def twice(value => Int) Int =
    value + value

def debug(message => Str) Unit
```

Rules:

- `name => Type` is allowed on function and method parameters only.
- A by-name argument expression is captured as a zero-argument closure.
- Reading the parameter evaluates that closure.
- By-name parameters are not memoized; each read evaluates the captured expression again.
- By-name parameters cannot be `vararg`.
- By-name argument expressions cannot contain non-local `return`, `break`, `continue`, or `try`.
- Use an explicit `fn() T` parameter when the caller should pass, store, or return the thunk itself.

Style:

- Use by-name parameters only for conditional-value APIs such as `assert`, `debug`, and `orElse`.
- Use `fn() T` for callbacks, schedulers, retry operations, event handlers, and stored work.

Forwarding rules:

```txt
def inner(value => Int) Int = value

def outer(value => Int) Int =
    inner(value)
```

- Passing a by-name parameter to a normal parameter evaluates it first.
- Passing a by-name parameter to another by-name parameter forwards the delayed expression.
- Reading a by-name parameter in any other expression evaluates it immediately.

If a callee needs one evaluation, bind the value explicitly:

```txt
def cached(value => Int) Int {
    item = value
    item + item
}
```

Core fallback APIs use by-name parameters so fallback work only runs on the
fallback branch. Mapper callbacks such as `map`, `flatMap`, `mapLeft`, and
`mapError` are ordinary function values; only the callback body is conditional
on the container branch:

```txt
value = maybe ?? expensiveDefault()
result = maybe.toResult(makeError())
next = result.orElse(recover())
mapped = maybe.map(value => value + 1)
leftMapped = either.mapLeft(error => error.toStr())
```

Classes, shapes, and named objects declare methods directly in their declaration
bodies, after storage fields and constructors. Declared unions put shared
behavior in an extension block:

```txt
class Counter {
    value Int
    def inc() Int = this.value + 1
}
```

Extension methods attach receiver-call syntax from outside the target type's
own implementation:

```txt
ext Counter {
    def doubled() Int = this.value * 2
}

counter = Counter { value: 3 }
println(counter.doubled())
```

Extension rules:

- extension blocks use `ext TypeName { def method(...) ... }`
- extension targets may be classes, shapes, declared unions, interfaces, or built-in primitive types such as `Int`, `Float`, `Bool`, `Str`, and `Rune`
- extension targets cannot be named objects, annotations, or individual union alternatives
- extension blocks cannot declare constructors
- a module may declare multiple `ext` blocks for the same target type
- extension methods use the same call syntax as regular methods
- `this` is the extended receiver
- extension methods can access only the visible members available from the extension module
- extension methods are visible in their declaring module and in files that use that module with `use module/*`
- extension visibility is file-local; using a module that itself uses extensions does not re-export those extension methods

```txt
use model/user/{User}
use model/user_extensions/*

user User = User { name: "Ada" }
label = user.displayName()
```

Custom constructors are class-only and use a dedicated `new(...)` declaration in
the class body.

- `new(...)` declares constructor inputs
- `new(...) { body }` declares a block-bodied constructor
- `new(...) = expression` declares an expression-bodied constructor
- shape, union alternative, object, annotation, and interface declarations cannot define custom `new` constructors
- constructor parameters use `name Type`, with optional defaults such as `age Int = 0`
- constructor defaults may appear anywhere
- `Type { field: value }` constructs by matching constructor parameters by field name
- `Type(value)` constructs by filling constructor parameters positionally by declaration order
- named construction may omit any constructor parameter with a default
- positional construction fills a declaration-order prefix and may stop only when every remaining parameter has a default
- positional construction never skips an earlier parameter
- constructor parameters may end with one variadic vector parameter such as `items [Str] vararg`
- `internal new(...) { body }` declares a constructor callable only from the same module
- `private new(...) { body }` declares a private constructor
- each explicit class constructor must initialize every field that does not have a field initializer, or delegate to another constructor
- `this(...)` inside a constructor delegates positionally to another constructor of the same class
- `this { field: value }` inside a constructor delegates with construction fields to another constructor of the same class
- delegating constructors use expression bodies, for example `new(label Str) = this { name: label }`
- direct and indirect constructor-delegation cycles are rejected
- in a class declaration, `new(...)` declares a constructor; in expression position, `new(...)` performs contextual construction
- contextual `new(...)` is not constructor delegation; use `this(...)` or `this { ... }` to delegate
- explicitly targeted class call sites use braces for construction fields, for example `Person { name: "Ada", age: 10 }`
- explicitly targeted class call sites use parentheses for positional arguments, for example `Person("Ada", 10)`
- contextual class call sites use the target-selection forms defined in [Construction](#construction)
- `this` is the instance receiver
- instance fields on classes and named objects may be accessed bare when they are not shadowed
- use `this.field` when a parameter/local shadows a field, for example `this.age`
- member order does not affect validity or semantics
- canonical formatting places storage fields first, constructors next, and methods last
- declared-union alternatives contain fields only; put shared methods in `ext UnionName`
- source code may interleave fields, constructors, and methods; a formatter should restore canonical order

```txt
class Person {
    age Int
    name Str

    new(age Int, name Str) {
        this.age = age
        this.name = name
    }

    new(age Int) = this(age, "unknown")

    new(name Str) = this {
        age: 0
        name: name
    }
}
```

Variadic constructor parameters collect positional arguments into a `[T]`
inside the constructor body:

```txt
class Path {
    segments [Str]
    new(segments [Str] vararg = ["tmp"]) {
        this.segments = segments
    }
}

path Path = Path("usr", "local", "bin")
named Path = Path { segments: ["etc", "hosts"] }
empty Path = Path()
```

## Lambdas

Accepted lambda parameter forms are deliberately small:

```txt
() => expr
x => expr
_ => expr
(x) => expr
(x, y) => expr
(x Int) => expr
(x Int, y Int) => expr
(_) => expr
(x, _) => expr
(_ Int, value Int) => expr
```

Typed single-parameter lambdas must use parentheses, so write
`(x Int) => x + 1`, not `x Int => x + 1`. Parenthesized parameter lists
must also be either fully typed or fully untyped; `(x Int, y) => ...` is
invalid. Plain `(x, y) => ...` always means two parameters.

Single-parameter lambda:

```txt
x => x + 1
```

Explicitly typed lambda:

```txt
(x Int) => x + 1
```

Multi-parameter lambda:

```txt
(left Int, right Int) => left + right
```

Tuple-destructuring inside a one-argument lambda:

```txt
pairs.map(pair => {
    let (key, value) = pair
    key + value
})

pairs.map(pair => {
    let (key, _) = pair
    key
})
```

Class or anonymous-shape destructuring inside a lambda:

```txt
users.map { user =>
    let { name, age } = user
    "$name is $age"
}
```

Lambda parameters cannot use `let` destructuring. If a lambda receives a tuple,
class, or anonymous-shape value, name the parameter normally and destructure it
inside the body:

```txt
pairs.mapWithIndex((pair, index) => {
    let (x, y) = pair
    "$index: ${x + y}"
})

source.combine((name, pair) => {
    let (x, y) = pair
    "$name: ${x + y}"
})

source.combine((left, right) => {
    let (a, b) = left
    let (x, y) = right
    a + b + x + y
})
```

Rules:

- `_` inside an explicit lambda parameter list means "ignore this parameter slot"
- `_` is not a readable value, so `(_, value) => _ + value` is invalid
- `_ => expr` is valid as a one-parameter lambda whose parameter is ignored
- placeholder-expression lambdas such as `_ + 1` and `items.map(_ + 1)` are not supported
- tuple, class, and anonymous-shape values are destructured inside the lambda body with normal `let`
- `let` destructuring is not allowed in lambda parameter lists

Callable references can be passed where a function value is expected. They are
eta-expanded to the same forwarding lambda you would otherwise write:

```txt
def mapUser(user User) UserDto =
    UserDto { id: user.id, name: user.name }

dtos = users.map(mapUser)
# same as: users.map(user => mapUser(user))

mapper UserMapper = UserMapper()
dtos = users.map(mapper.mapUser)
# same as: users.map(user => mapper.mapUser(user))

dtos = users.map(this.mapUser)
# same as: users.map(user => this.mapUser(user))

dtos = users.map(User.toDto)
# named object method reference
```

Supported callable references:

- top-level function name
- bound instance method, such as `mapper.mapUser`
- bound `this` method, such as `this.mapUser`
- bound named-object method, such as `User.toDto`

Stored fields, computed getters, and methods share one member-name namespace.
A type cannot reuse a name across those categories, including through an
`ext` block, so member access and callable references never need
field-versus-method precedence:

```txt
class Amount {
    label Str

    def label() Str = this.label  # invalid: field/method collision
}
```

Ordinary methods may still overload one another. Getters cannot be overloaded
and cannot share a name with an ordinary method. A function-valued field remains
an ordinary field value and can be called normally.

Block lambda:

```txt
(x Int) => {
    next = x + 1
    next
}
```

After `=>`, a standalone lambda accepts exactly one body unit: an expression, a
statement, or a `{ ... }` block. If the body is an expression, normal multiline
expression continuation rules apply:

```txt
mapper = item =>
    item +
        1
```

To put multiple statements inside the lambda, use an explicit block. Lambda
scope is determined by tokens and braces, never by indentation:

```txt
# `next` belongs to the enclosing block despite the misleading indentation.
mapper = item =>
    item + 1
    next = 2

# Both statements belong to the lambda.
mapper = item => {
    next = item + 1
    next * 2
}
```

A formatter may correct suspicious indentation, but indentation alone does not
change scope or make a program invalid.

Trailing lambda call syntax is also allowed when passing a lambda as an argument. The trailing brace body must contain an explicit lambda head with `=>`. Layout before that head is insignificant:

```txt
items.map { x => x + 1 }

items.map {
    x => x + 1
}

items.repeat { () => 5 }

runner.zero { () => 26 }

items.zipMap { (left, right) => left + right }

items.forEach { x =>
    next = x + 1
    println(next)
}

items.map { (x Int) =>
    x + 1
}

items.zipMap { (left,
    right) => left + right }
```

Headless trailing blocks are rejected. Write the zero-argument lambda head explicitly:

```txt
# invalid
runner.zero {
    26
}

# valid
runner.zero { () => 26 }
```

If a callback is passed alongside ordinary arguments, include it in the same
parenthesized argument list. Do not write a trailing block after an already
completed `(...)` call; that would imply currying or calling the result of the
first call.

```txt
processNamed("compares values", { () => println("inside callback") })
```

In a control-flow header, trailing brace-call syntax is disabled only at the
header's outer nesting level so the header body remains unambiguous. Ordinary
expression syntax is restored inside explicit delimiters such as parentheses,
argument lists, indexes, and collection literals. Nested construction and an
explicitly grouped trailing-lambda call are therefore valid:

```txt
if positive(Box { value: 1 }) {
    println("positive")
}

if (check { () => true }) {
    println("ready")
}
```

The same nesting rule applies to `while` conditions, `for` iterables, and
`match` scrutinees.

Trailing brace call syntax on non-constructor calls is only for lambda arguments.
Constructor braces fill constructor inputs by field name, so declared-union
payloads use braces and unary payload shorthand uses parentheses:

```txt
maybeOrder = Some(Order { id: 7 })
namedMaybeOrder = Some { value: Order { id: 7 } }
```

Use an explicit lambda when mapping with a `match`:

```txt
options.map(value => match value {
    case SomeX { value as x } => x + 1
    case NoneX => 0
})
```

Nested blocks are also valid expressions:

```txt
a1 = {
    1 + 7
}

v := {
    a = 5
    {
        a + 1
    }
}
```

Rules:

- braced blocks may appear as standalone statements or as expressions
- block expressions evaluate to the value of their last statement
- successive statements in a braced block must be separated by a newline; a
  closing `}` may immediately follow the final statement
- named function and method block bodies may be written as `def name(...) { ... }` or `def name(...) = { ... }`; without an explicit return type, the direct block returns `Unit` and the equals block infers its result type
- if you want a block value, the last statement must be value-producing
- value-producing tail forms include ordinary expressions, `if / else`, `match`, and `for ... yield`
- blocks can nest arbitrarily

## Classes, Shapes, Objects, Interfaces, Declared Unions

Class:

```txt
class Box[T] with Named {
    value T
    def label() Str = "box"
}
```

When a class, shape, or named object implements an interface method inside
its body, it uses an ordinary `def` method declaration.

Named object:

```txt
object MathBox {
    value Int = 5

    def valuePlusOne() Int = this.value + 1
    def double(value Int) Int = value * 2
}

box = MathBox
answer = box.valuePlusOne()
```

`object Name { ... }` declares one named object type and one value `Name`.
The expression `Name` evaluates to that value, so named objects can be passed to functions, stored in locals, and called through later like any other value.
Named objects cannot be constructed with `Name()` or `Name {}`; reference `Name` directly.

Anonymous objects use an expression form:

```txt
value = object {
    count Int = 4
    label Str = "items"

    def describe() Str = this.label + ": " + this.count.toStr()
}
```

Rules:

- every anonymous-object field has an initializer
- anonymous-object fields are immutable; use a named class for owned mutable state
- field and method order does not affect validity; canonical formatting places fields before methods
- anonymous objects cannot declare `new` constructors
- fields and methods are statically typed and use ordinary member access
- `this.field` and unqualified `field` are both available inside methods

To create an anonymous nominal interface implementation, use `object with`:

```txt
greeter Greeter = object with Greeter {
    greeting Str = "hello"

    def greet() Str = greeting
}
```

`object` and `object with` use the same body grammar. Both accept initialized
immutable fields and methods; `with` only adds the interfaces implemented by
the synthesized nominal object.

An interface name followed directly by braces is not anonymous implementation
syntax. `Greeter { ... }` is rejected because interfaces cannot be constructed.

Another class example:

```txt
class Amount with Named {
    value Int
    label Str
    def label() Str = this.label
}
```

Interfaces:

```txt
interface Named {
    def label() Str
}
```

Interface implementation and inheritance lists use one `with`, followed by
comma-separated interface types:

```txt
class Service with Readable, Writable {
}

value = object with Readable, Writable {
    def read() Str = "value"
    def write(value Str) Unit = ()
}
```

Anonymous implementations start with `object with`. Repeating `with`, such as
`object with Readable with Writable`, is invalid; write `object with Readable,
Writable` instead. Expression-level `shape with` is not supported.

Interfaces may also provide default methods by attaching a body:

```txt
interface Named {
    def label() Str
    def greeting() Str = "Hello " + this.label()
}
```

Methods that satisfy an interface just use ordinary method declarations:

```txt
interface Named {
    def label() Str
}

class Box with Named {
    def label() Str = "box"
}
```

Anonymous interface implementation expressions:

```txt
handler = object with Reader, Closer {
    def read() Str = "x"
    def close() Unit = ()
}
```

Declared unions:

```txt
type OptionX[T] =
    class SomeX { value T }
    | object NoneX {}

ext OptionX[T] {
    def isSet() Bool = match this {
        case SomeX(_) => true
        case NoneX => false
    }
}
```

Declared alternatives are data-only. Class and shape alternatives may declare
payload fields; object alternatives are singleton values and therefore have no
instance fields. Alternatives do not declare methods or custom constructors.
Put behavior for the complete union in `ext UnionName` and distinguish
alternatives with `match`.

## Calls

Normal call:

```txt
add(1, 2)
```

Named arguments:

```txt
format(prefix = "item", value = 5)
```

Explicit argument expressions are evaluated exactly once in written source
order. Their resulting values are then bound to parameters by position or name.
For a member call, the receiver is evaluated before any argument expression:

```txt
combine(second = mark(2), first = mark(1))
# evaluates mark(2), then mark(1), then calls combine(firstValue, secondValue)

makeService().send(makeRequest())
# evaluates makeService(), then makeRequest(), then invokes send
```

Named and contextual construction follow the same rule: supplied field
expressions evaluate in written order, even though their values are placed into
constructor inputs or shape fields in declaration order. Defaults are supplied
after explicit arguments have been evaluated. By-name parameters remain lazy;
their expressions evaluate only when the callee reads them.

Methods are called explicitly:

```txt
adder Adder = Adder(5)
adder.add(7)
```

Range construction is explicit:

```txt
Range(10, 0, -1)
```

## Vectors, Arrays, Maps, Tuples

Vector literal:

```txt
[1, 2, 3]
["a", "b"]
[0, ...items, 5, ...more]
copy = [...items]
```

`...items` inside a vector literal copies each element from an iterable into the
new vector. The copy is shallow: element values are reused, but the outer vector
is new. Multiple spreads may appear in one literal. A map is not a vector-spread
source; use `map.entries()` when a vector of `(key, value)` tuples is wanted:

```txt
parts = [1, 2]
more = [3, 4]
combined = [0, ...parts, ...more, 5]

entryVector [(Str, Int)] = [...map.entries()]
```

`LinkedList[T]` is a mutable doubly linked list. Use it when adding or removing
values at either end is more important than random-access performance:

```txt
queue LinkedList[Int] = LinkedList {}
queue.add(10)
queue.add(20)

first Option[Int] = queue.at(0)
removed Option[Int] = queue.removeFirst()

populated = LinkedList(1, 2, 3)
```

`at`, `first`, `last`, `removeFirst()`, and `removeLast()` are safe and
return `Option[T]`. Indexed mutations return `Result` with an `InvalidIndex`
that records the rejected index and the collection size. `setAt` returns the
replaced value, `removeAt` returns the removed value, and `insertAt` accepts
indices from zero through the current size and returns `Unit`:

```txt
shape InvalidIndex {
    index Int
    size Int
}

previous Result[Int, InvalidIndex] = queue.setAt(0, 20)
inserted Result[Unit, InvalidIndex] = queue.insertAt(1, 30)
removedAt Result[Int, InvalidIndex] = queue.removeAt(0)
```

`LinkedList {}` is named empty construction and `LinkedList()` is its positional
equivalent; non-empty values use `LinkedList(...)`. The old indexed `get` and
`remove` methods are not part of the collection API.

Array construction:

```txt
ints Array[Int] = Array.ofInt(3)       # [0, 0, 0]
floats Array[Float] = Array.ofFloat(3) # [0.0, 0.0, 0.0]
bools Array[Bool] = Array.ofBool(3)    # [false, false, false]
texts Array[Str] = Array.ofStr(3)      # ["", "", ""]
runes Array[Rune] = Array.ofRune(3)    # default NUL rune values

filled Array[Int] = Array.fill(3, 7)
generated Array[Int] = Array.generate(3, idx => idx * 2)
```

Arrays have fixed size and always contain initialized values. Use
`Array.generate` when each slot should be produced independently. Arrays expose
`at(index)` and `setAt(index, value)`, but not insertion or removal because
their size cannot change.

Array elements can also be constructed directly:

```txt
values Array[Int] = Array(1, 2, 3)
boxes Array[Box] = Array(Box(1), Box(2))
takeArray(Array(4, 5, 6))
```

Vectors expose the optional `first` and `last` getters plus `at`, `setAt`,
`insertAt`, and `removeAt` with the same safe return types as LinkedList. Vector
and Array bracket access remains available as the explicit unsafe alternative:

```txt
first = values.first ?? fallback
last = values.last ?? fallback
```

`take(count)` returns a new vector containing at most the first `count` values;
a non-positive count produces an empty vector. `sort` mutates the vector.
Values implementing `Ordered[T]` can use the parameterless form; an explicit
comparator function provides an ad hoc ordering:

```txt
values.sort()
values.sort((left, right) => left.score - right.score)
firstTen = values.take(10)
```

`Vector`, `LinkedList`, and `Set` provide `makeStr`. The one-argument form uses
each value's normal string rendering. The two-argument form accepts an explicit
formatter:

```txt
names.makeStr(", ")
evidence.makeStr(", ", value => value.label())
```

A comparator returns a negative value when `left` belongs before `right`, zero
when they compare equally, and a positive value when `left` belongs after
`right`.

`flatten()` removes one iterable layer. The vector element type must implement
`Iterable[X]`; this includes nested vectors, sets, and `Option[X]`. An absent
option contributes no value:

```txt
present [Int] = [Some(1), None, Some(3)].flatten() # [1, 3]
nested [Int] = [[1, 2], [3]].flatten()             # [1, 2, 3]
```

Map construction:

```txt
entries [Str : Int] = ["a": 1, "b": 2]
empty [Str : Int] = []
value Option[Int] = entries["a"]
allKeys [Str] = entries.keys()
allValues [Int] = entries.values()

totals [Str : Int] = ["Ada": 10]
totals["Ada"] += 5
totals["Ada"] -= 2

defaults [Str : Int] = ["port": 80, "secure": 0]
overrides [Str : Int] = ["port": 443]
copy = [...defaults]
merged = [...defaults, "retries": 3, ...overrides]
```

Indexed map compound assignment updates an existing entry and supports the same
`+=`, `-=`, `*=`, `/=`, and `%=` operators as ordinary mutable numeric targets.
It panics when the key is absent; use `map[key] := initial` when insertion is
intended.

`[K : V]` is the concise map type syntax and is equivalent to `Map[K, V]`.
The colon belongs to type grammar here; it does not construct a pair value.

Non-empty map literals use `[key: value, ...]`. Keys are expressions, so
computed and tuple keys do not need a separate marker:

```txt
dynamic = "name"
scores [Str : Int] = [dynamic: 10, makeKey(): 20]
positions [(Int, Int) : Str] = [(10, 20): "start"]
```

Map entries and spreads are comma-separated and may be interleaved. Spreading
a map copies its entries into a fresh map. Parts are evaluated from left to
right, and a later entry or spread replaces an earlier value with the same key.

Map entries cannot be mixed with vector items. The spread source determines the
collection family when a literal contains only spreads: `[...vector]` is a vector
and `[...map]` is a map. Multiple spread sources must belong to the same family.
A map cannot be spread directly into a vector, and an iterable/vector cannot be
spread into a map.

Collection literals are distinguished by their contents and spread sources:

```txt
[]                    # contextual empty vector or map
[value, ...]          # vector
[key: value, ...]     # map
[...vector]           # vector copy
[...map]              # map copy
```

Trailing commas are permitted only in bracket collection literals and brace
construction literals:

```txt
values = [1, 2,]
lookup = ["left": 1, "right": 2,]
point = { x: 10, y: 20, }
```

They are rejected in every other comma-separated form, including declarations,
generic clauses, calls, function types, tuples, and patterns:

```txt
def process(name Str, amount Int,) Unit {}  # invalid
fn(Int,) Int                                # invalid
Box[Int,]                                  # invalid
(1, 2,)                                    # invalid
let (left, right,) = pair                  # invalid
```

An empty `[]` literal contains no elements that identify its collection family
or type arguments. It therefore requires an immediate expected vector or map
type:

```txt
names [Str] = []                 # empty vector
counts [Str : Int] = []          # empty map

def emptyNames() [Str] = []
def emptyCounts() [Str : Int] = []

consumeNames([])                  # valid when the parameter is [Str]
consumeCounts([])                 # valid when the parameter is [Str : Int]

values = []                       # invalid: collection type is unknown
```

The compiler does not default `[]` to a vector, infer `[Any]` or `[Any : Any]`,
or infer its type from later mutations. Only `[T]` and `[K : V]` provide valid
contexts; `Set[T]`, `Array[T]`, and custom collection types retain their own
construction syntax. If overloaded vector and map parameters both match `[]`,
the call is ambiguous and requires an intermediate typed binding. The former
empty-map spelling `[:]` is not supported.

Map construction belongs only to bracket literals. The former brace forms,
including `Map { "key": value }` and `Map { [key]: value }`, are not
supported. Braces remain reserved for construction fields and shape literals.

Tuple literal:

```txt
(1, "x")
pair (Str, Int) = ("a", 1)
```

Tuples always contain at least two elements. The general trailing-comma rule
also rejects singleton tuple spellings:

```txt
value = (1,)                 # invalid; use 1
value (Int,) = ...           # invalid; use Int
let (value,) = tuple         # invalid
let (value, _, _) = tuple3   # valid full-arity extraction
```

`:` is not a general expression operator. It appears in map types, map
literals, and construction field lists:

- `[K : V]` separates the key and value types of a map
- `[key: value]` constructs a map entry
- `field: value` binds a value to a construction field

Tuple values inside field initializers should use tuple syntax:

```txt
holder = Holder {
    entry: ("a", 1)
}
```

## Statements

Main statement forms:

- value binding
- assignment / reassignment
- local function
- `if`
- `match`
- `for`
- `while`
- `defer`
- `return`
- `break`
- `continue`
- expression statement

Pure expression statements with no effect are rejected.

Standalone nested blocks are valid expression statements:

```txt
{
    println("xxx")
}
```

## `defer`

`defer` registers cleanup for the current callable. Deferred actions run in
LIFO order whenever the enclosing function, method, or lambda exits normally
or through a language-managed runtime error such as `panic`.

All pending deferred actions are attempted even when one of them fails. When
the callable body has already failed, that original diagnostic remains primary
and cleanup failures are attached as notes. On an otherwise successful exit,
the first cleanup failure is reported as the primary diagnostic and later
cleanup failures are attached as notes. A hard process abort is outside this
guarantee.

`defer` is not block-bound. A `defer` inside an inner `{ ... }` block still runs
when the enclosing callable exits.

A lambda has its own defer queue. A `defer` inside a lambda runs when that
lambda returns, not when the outer function returns.

Supported forms:

```txt
defer cleanup()

defer {
    println("closing")
}
```

Only a call expression or a block is allowed after `defer`. Deferred blocks may
not contain `return`, `break`, or `continue`.

## `if`

Statement form:

```txt
if value > 0 {
    println("positive")
} else {
    println("non-positive")
}
```

Pattern-test form:

```txt
if let Some { value as item } = maybeValue {
    println(item)
}
```

`if let` also accepts the shorthand for the success case:

```txt
if let item <- maybeValue {
    println(item)
}
```

Runtime type patterns also work in `if let`:

```txt
if let worker Worker = value {
    println(worker)
}

if let _ Worker = value {
    println("value is a Worker")
}
```

Runtime type tests use `is`; direct negative tests use `is not`:

```txt
if value is Str {
    println(value.size)
}

if value is not Worker {
    return "not a worker"
}
```

`not` is contextual after `is`; it is not a second general Boolean-negation
operator. `value is not Worker` means exactly `!(value is Worker)`. The right
side is always a type reference, so value comparisons continue to use `!=` or
`!==`; reference identity compares the operands' `referenceId` values.

`is` is non-associative. A type test has the grammar
`comparison ["is" ["not"] type]`, so chained tests are rejected:

```txt
value is Str                     # valid
value is not Worker              # valid
value is Str is Any              # invalid
value is not Worker is Any       # invalid
(value is Str) && otherCheck     # valid
```

Inside the successful branch, an immutable local binding or parameter tested
directly by name is narrowed to the tested type. Parentheses and a leading `!`
are recognized. When the opposite branch exits, the positive narrowing remains
available afterward:

```txt
def size(value Any) Int {
    if value is not Str {
        return 0
    }

    value.size
}
```

Inside the `else` branch of `value is not Type`, the value is narrowed to
`Type`. The true branch keeps its original type because Lume does not currently
represent negative types. The parenthesized `!(value is Type)` spelling remains
legal and is useful when negating a larger condition, but `is not` is canonical
for a direct negative type test.

This narrowing is intentionally local and conservative:

- mutable bindings are not narrowed, because another read may observe a different value
- by-name parameters, member reads, indexes, and arbitrary expressions are not narrowed
- narrowing is propagated across top-level `&&` condition clauses, but nested
  compound Boolean expressions do not currently combine narrowing facts
- the checker does not currently infer negative types or report unreachable type-test branches

Runtime type arguments are erased. Generic runtime tests and type patterns must
name only the outer type: `value is Box` and `_ Box` are valid, while
`value is Box[Int]` and `_ Box[Int]` are rejected.

`if let` is intended for refutable matches. If the compiler can prove the
pattern always succeeds for the scrutinee type, it rejects the construct and
asks you to use plain `let` instead.

When the payload needs more destructuring, prefer doing that on the next line inside the branch:

```txt
if let Some { value as pair } = maybePair {
    let (x, y) = pair
    println(x)
    println(y)
}
```

Statement form may omit `else`:

```txt
if value > 0 {
    println("positive")
}
```

Expression form must include `else`, because it has to produce a value on both
paths:

```txt
result = if value > 0 {
    1
} else {
    0
}
```

Statement `if`, expression `if`, and `while` use the same condition grammar.
Boolean expressions keep the ordinary operator precedence in every context,
so a Boolean-only condition such as `a || b && c` means `a || (b && c)`.
Extraction clauses may be mixed with Boolean segments in either order by
writing `&& let`:

```txt
if ready && let user <- maybeUser && user.active {
    println(user.name)
}

if ready &&
    let user <- maybeUser &&
    user.active {
    println(user.name)
}

if let user <- maybeUser && ready && user.active {
    println(user.name)
}

name = if ready && let user <- maybeUser {
    user.name
} else {
    "unknown"
}
```

When a condition contains an extraction clause, every disjunction in its
Boolean segments must be parenthesized. This makes the sequential clause
boundary explicit:

```txt
if (cached || ready) && let user <- maybeUser {
    println(user.name)
}

if let user <- maybeUser && (user.active || overrideEnabled) {
    println(user.name)
}
```

This visually familiar but misleading form is rejected:

```txt
if cached || ready && let user <- maybeUser {  # invalid
    println(user.name)
}
```

Clauses are evaluated from left to right and short-circuit on the first false
Boolean expression or failed pattern. A binding introduced by a `let` clause is
available to every later clause and to the successful branch, but not to
`else`. An outer `&& let` starts an extraction clause; every other `&&` remains
part of its ordinary Boolean expression.

Invalid:

```txt
result = if value > 0 {
    1
}
```

Brace-delimited branches are the preferred `if` form. `else` does not require `:`.

## Irrefutable and Refutable Bindings

Plain `let` is the irrefutable binding form. Use it when the pattern is known
to match:

```txt
pair (Int, Int) = (1, 2)
let (left, right) = pair
```

If the pattern can fail, plain `let` without `else` is rejected. Add an `else`
fallback for recoverable refutable binding.

`let ... else` is the refutable binding form with an explicit fallback path.
The fallback may either exit control flow or supply values for the bindings:

```txt
let Some { value as item } = maybeValue else {
    return Err("missing")
}

let Some(item) = maybeValue else defaultItem
```

For success-carrying values, `<-` is shorthand for the success case:

```txt
let item <- maybeValue else {
    return Err("missing")
}
```

This is equivalent to:
- `let Some { value as item } = maybeValue else { ... }` for `Option[T]`
- `let Ok { value as item } = maybeResult else { ... }` for `Result[T, E]`
- `let Right { value as item } = maybeEither else { ... }` for `Either[L, R]`

The shorthand requires the source type to be statically known as one of these
forms. If the source type is unknown, use an explicit pattern instead.

Type-pattern binding is also supported:

```txt
let worker Worker = value else {
    return Err("wrong kind")
}

let _ Worker = value else {
    return Err("wrong kind")
}
```

Vector-pattern binding is supported for `Vector[T]` / `[T]` values:

```txt
let [left, right] = values else {
    return Err("expected exactly two values")
}

let [name Str, age Int] = valuesOfAny else {
    return Err("wrong value shape")
}

let [head, ...tail] = values else {
    return Err("empty vector")
}

let [left, right, ...] = values else {
    return Err("expected at least two values")
}

let [...all] = values
```

Vector pattern rules:

- `[a, b]` matches exactly two elements.
- `[]` matches an empty vector.
- `[a, ...rest]` matches one or more elements and binds `rest` as `[T]`.
- `[...rest]` matches any vector and binds a shallow vector tail copy as `[T]`.
- Only one `...rest` is allowed, and it must be last.
- Bare `...` ignores the remaining elements; `..._` is the equivalent explicit spelling.
- Vector patterns are for `Vector[T]` / `[T]`; `Array[T]` is not part of this pattern surface.

Grouped refutable bindings share one fallback:

```txt
let {
    Some { value as left } = maybeLeft
    Some { value as right } = maybeRight
} else {
    return Err("missing")
}
```

When the fallback supplies a value, it is evaluated lazily only after a failed
match. A pattern with one binding accepts one assignable fallback value. A tuple
is still one value when that binding itself has a tuple type:

```txt
let item <- maybeItem else defaultItem

let Some(pair) = maybePair else (1, 2)
```

A pattern or grouped extraction with multiple bindings requires a shape whose
fields exactly match the introduced local names:

```txt
let User { name, label as title } = value else {
    name: "Unknown"
    title: "Untitled"
}

let User { name, value } = candidate else {
    name = defaultName
    value = defaultValue
    new { name, value }
}

let {
    user <- maybeUser
    account <- maybeAccount
} else new {
    user: fallbackUser
    account: fallbackAccount
}
```

Shape values map by local binding names, including aliases (`title` above), not
by source field names. The shape field set must match exactly, and every field
must be assignable to its corresponding binding type. A tuple is never
distributed positionally across multiple bindings:

```txt
let User { name, value } = candidate else (defaultName, defaultValue)
# error: use a shape with fields {name, value}
```

When a pattern aliases a whole value and also binds components inside that
value, its fallback must exit control flow. A value fallback could otherwise
initialize the whole and its components independently, breaking the
relationship expressed by the successful pattern:

```txt
let User { name } as user = candidate else return       # valid
let Some(User { name }) = candidate else "Unknown"      # valid: one binding

let User { name } as user = candidate else {            # invalid
    name: "Unknown"
    user: User("Ada")
}
```

To recover with another whole value, select it first and then destructure it:

```txt
let user User = candidate else User("Unknown")
let { name } = user
```

`let ... else` remains statement-oriented:
- the pattern is matched against the right-hand value
- if the match succeeds, bindings remain visible after the statement
- if the match fails, the `else` body is evaluated lazily
- the fallback must either initialize every introduced binding or exit the current control-flow path with `return`, `break`, `continue`, or a call whose return type is `Never`

Success-case extraction shorthand in `let` always requires an explicit
fallback, even when the source expression visibly constructs a successful
case:

```txt
let item <- Some(5) else panic("expected value")  # ok

maybe Option[Int] = Some(5)
let item <- maybe          # error: '<-' extraction requires else
```

For assertive extraction, write an explicit `panic(...)` fallback:

```txt
let Some { value as item } = maybeValue else panic("expected Some")
let item <- maybeValue else panic("expected value")
```

Grouped assertive extraction uses the same `let { ... } else` form:

```txt
let {
    Some { value as left } = maybeLeft
    Some { value as right } = maybeRight
} else panic("expected both values")
```

Use the runtime/prelude `assert(...)` function for plain boolean assertions:

```txt
assert(split.size == 3)
assert(split.size == 3, "split must have 3 parts")
```

The first argument must be `Bool`. When the condition is `false`, `assert`
panics. The optional second argument is the panic message.

Propagation form:

```txt
item = try maybeValue
```

`try` unwraps the success side of:
- `Option[T]`
- `Result[T, E]`
- `Either[L, R]`

and returns early with the original failure value when the source is empty / error / left.

`try` is only valid when the enclosing callable returns a compatible propagation
type:
- `Option[T]` may propagate from any `Option[...]` return type
- enclosing `Result[T, E]` may propagate from `Result[..., E2]` when `E2` is assignable to `E`
- enclosing `Either[L, R]` may propagate from `Either[L2, ...]` when `L2` is assignable to `L`

The success type may differ; the propagated failure side must still be compatible.

Failure mapping is ordinary container transformation before `try`:

```txt
user = try maybeUser.toResult(AppError.NotFound(id))
row = try Db.query(id).mapError { err => AppError.Db(err) }
value = try sourceEither.mapLeft { left => AppError.FromLeft(left) }
```

`try` propagates the value it receives. If the source has the wrong failure type,
transform the container first:

- `Option[T].toResult(error)` converts absence into `Err(error)`.
- `Result[T, E].mapError(f)` maps `Err(E)` into another error type.
- `Either[L, R].mapLeft(f)` maps `Left(L)` into another left type.

When the chain gets visually noisy, split before the mapping call:

```txt
row = try Db.query(id)
    .mapError { err => AppError.Db(err) }
```

Extract-or-fallback form:

```txt
value = wrapped ?? fallback
```

`??` unwraps the success side of `Option[T]`, `Result[T, E]`, or
`Either[L, T]`. If the wrapped value is empty / error / left, the right-hand
fallback is evaluated lazily and used instead. The fallback must be assignable
to the extracted success type, or have type `Never`.

```txt
name = maybeName ?? "unknown"
row = queryRow() ?? defaultRow()

value = maybeValue ?? {
    println("missing value")
    0
}
```

Control-flow expressions have type `Never`, so they work naturally as
fallbacks:

```txt
user = findUser(id) ?? return
user = findUser(id) ?? return Err(UserNotFound(id))

for request <- requests {
    user = findUser(request.userId) ?? continue
    process(user, request)
}

while true {
    item = queue.next() ?? break
    process(item)
}
```

`return` targets the current callable. `break` and `continue` require an
enclosing loop and cannot jump across lambda boundaries. `continue` and
`break` inside `for ... yield` are valid only for
iterable comprehensions; `Option`, `Result`, and `Either` comprehensions have no
“skip item” or “early-exit item” state.

`try` and `??` intentionally do different jobs:

- `try` propagates the original failure.
- `??` discards/replaces the failure with an explicit fallback.

The prefix optional-wrapping operator `^` is an exact shorthand for `Some`:

```txt
optional Int? = ^5
inferred = ^"ready" # Option[Str]
```

In a pattern, the same spelling performs the corresponding `Some` match:

```txt
let ^value = optional else return
```

It always constructs `Option`; it never selects `Ok` or `Right` from the
surrounding type. Construct those alternatives explicitly:

```txt
result Result[Int, DbError] = Ok(5)
either Either[AppError, Int] = Right(42)
```

An expected `Option[T]` may still supply `T` as context for the operand. It
refines the payload type without changing the meaning of `^`:

```txt
point Point? = ^new(10, 20)
```

The operator evaluates its operand once and wraps exactly one layer. Repeating
it explicitly wraps multiple layers:

```txt
nested Option[Option[Int]] = ^^5
nestedMissing Option[Option[Int]] = ^None
```

`^` does not flatten an existing `Option`, `Result`, or `Either`. Use `map`,
`flatMap`, `try`, or `??` when transforming or extracting an existing lifted
value.

Unsafe extraction uses postfix `!`:

```txt
value = wrapped!
```

It extracts the success value from `Option[T]`, `Result[T, E]`, or
`Either[L, T]` and panics when the value is empty / error / left. Use it only
when failure is a programming error; prefer `try`, `??`, or `let ... else` for
recoverable control flow.

`!` is a normal postfix operator. Whitespace before it is optional, and calls,
indexing, member access, and further extraction may follow it directly:

```txt
item = values[index]!
item = values[index] !
name = wrapped!.name
result = callback!()
first = wrappedVector!.at(0)!
entry = wrappedMap!["key"]!
nestedValue = nested!!
```

Each postfix `!` extracts exactly one layer, so `nested!!` means
`(nested!)!`. Prefix `!` remains Boolean negation; position distinguishes the
two forms, and `!maybeReady!` means `!(maybeReady!)`.

`!=` and `!==` remain indivisible binary operators. Therefore
`maybe!==expected` is strict inequality, while extraction followed by
equality is written `maybe! == expected`. The formatter places spaces around
binary operators.

Multiple dependent unwraps can be written as sequential `let ... else` / `try`
statements or as a grouped `let` block with `else`:

```txt
left = try maybeLeft

let Some { value as right } = maybeRight else {
    return Err("missing")
}

let {
    Some { value as left } = maybeLeft
    Some { value as right } = maybeRight
} else {
    return Err("missing")
}
```

`if let` also supports a grouped form:

```txt
if let {
    Some { value as left } = maybeLeft
    Some { value as right } = maybeRight
} {
    println(left + right)
}
```

A headless record pattern remains one condition rather than a grouped clause
block when its closing brace is followed by `=` or `<-`:

```txt
if let { age: 18 } = user {
    println("eighteen")
}

while let { status: Active } = user {
    process(user)
}
```

And grouped clauses can use `<-` too:

```txt
if let {
    left <- maybeLeft
    right <- maybeRight
} {
    println(left + right)
}
```

Condition clauses can be chained so later clauses can use earlier bindings.
An outer `&& let` begins another extraction clause. Boolean segments retain
ordinary operator precedence, but any `||` in a condition that also contains
an extraction clause must be grouped explicitly. Newlines after `&&` do not
change clause recognition:

```txt
if let Some { value as left } = maybeLeft && let Ok { value as right } = compute() && right > left {
    println(left + right)
}

if ready && let left <- maybeLeft && left > 0 {
    println(left)
}

if ready &&
    let left <- maybeLeft &&
    left > 0 {
    println(left)
}

if (cached || ready) && let user <- maybeUser && (user.active || overrideEnabled) {
    println(user.name)
}
```

Extraction clauses are joined with `&&`; `||` remains an ordinary Boolean
operator inside a Boolean segment but requires parentheses in a mixed
condition.

## `for`

Simple loop:

```txt
for item <- [1, 2, 3] {
    println(item)
}
```

Range loop:

```txt
for i <- Range(0, 10) {
    println(i)
}
```

`Range(start, end)` is start-inclusive and end-exclusive. With two arguments it automatically chooses a step of `1` or `-1` based on the bounds, and `Range(start, end, step)` allows an explicit step.

Generator heads normally bind one plain identifier, or `_` when the item is
intentionally ignored:

```txt
for row <- rows {
    println(row)
}

for _ <- events {
    incrementCount()
}
```

Use `for let` for explicitly marked irrefutable patterns. Tuple and shape
destructuring are the common forms:

```txt
for let (x, y, char) <- rows {
    println(char)
}
```

The same rule applies to class and anonymous-shape values. Shape
destructuring matches by field name, not by position:

```txt
for let { name, location } <- users {
    println(name, location)
}

for let { location as loc, name } <- users {
    println(name, loc)
}
```

Refutable logic goes in the loop body:

```txt
for maybeItem <- items {
    let Some { value as item } = maybeItem else {
        continue
    }
    println(item)
}
```

These generator heads are invalid:

```txt
for (x, y) <- pairs { ... }
for { name, age } <- users { ... }
for Some { value as item } <- values { ... }
for let Some { value as item } <- values { ... }
for let worker Worker <- values { ... }
for item Int <- items { ... }
```

Yield form:

```txt
items = for item <- [1, 2, 3] yield {
    item * 2
}
```

The short yield form uses the same generator-binding grammar as an ordinary
loop. Use `for let` for an irrefutable tuple or shape pattern:

```txt
sums = for let (left, right) <- pairs yield left + right

names = for let { name } <- users yield name
```

This has the same generator capability as the grouped form:

```txt
sums = for {
    let (left, right) <- pairs
} yield left + right
```

Refutable generator patterns remain invalid in every form. Extract or match
inside the body instead.

Multi-clause yield form:

```txt
items = for {
    x <- [1, 2]
    doubled = x * 2
    y <- [10, 20]
} yield {
    doubled + y
}
```

`yield` also accepts a same-line expression:

```txt
items = for item <- [1, 2, 3] yield item * 2
```

`for ... yield` may also pull success values from lifted containers:
`Option[T]`, `Result[T, E]`, and `Either[L, T]`. The first generator source
chooses the result family:

```txt
maybeName Option[Str] =
    for user <- maybeUser yield user.name

total Result[Int, DbError] =
    for {
        left <- loadLeft()
        right <- loadRight()
    } yield left + right
```

For lifted comprehensions, every `<-` generator must use the same lifted
family. `Result` failure types and `Either` left types from later generators
must be assignable to the first generator's failure or left type. Convert
failures explicitly before the generator when needed:

```txt
value = for {
    row <- dbRow.mapError(err => AppError.Db(err))
    user <- decodeUser(row)
} yield user
```

Only these clause kinds are allowed inside `for { ... } yield`:

```txt
name <- source
let (x, y) <- iterable
let { name, age } <- iterable
name = expr
let (x, y) = pair
let { name, age } = user
```

Plain local bindings do not use `let`:

```txt
value = expr      # ordinary binding
let (x, y) = pair # destructuring
```

`let` clauses must be statically irrefutable:

```txt
values = for {
    pair <- pairs
    let (x, y) = pair
} yield x + y
```

Refutable `let ... else`, reassignment, mutation, and expression statements are
not clause forms. Put that logic in the body or use helpers such as `filterMap`:

```txt
result = items.filterMap(item => match item {
    case Some(value) => Some(value)
    case None => None
})
```

Mental model:

```txt
for      = pulls values from iterables; in yield form, also from lifted success values
let      = destructures irrefutable values, or exits early with `else`
match    = handles refutable cases
yield    = produces values
```

`for item <- items yield item * 2` lowers approximately to
`items.map(item => item * 2)`.

If `items` is `Option`, `Result`, or `Either`, the same spelling lowers to that
type's `map`.

Nested generators lower approximately through `flatMap` and `map`:

```txt
for {
    x <- xs
    y <- ys
} yield x + y
```

is approximately:

```txt
xs.flatMap(x => {
    ys.map(y => {
        x + y
    })
})
```

`break` and `continue` are valid in `while`, `for`, and iterable
`for ... yield`.
Inside iterable `for ... yield`, `continue` skips the current iteration without
producing a value, and `break` exits the current generator loop.

`break` and `continue` are invalid inside `Option`, `Result`, and `Either`
comprehensions. Those families lower through `map` / `flatMap`, not real
iteration, and they do not have skip or early-exit states. Choose absence or
failure explicitly with `match`, `map`, `flatMap`, `None`, `Err`, or `Left`.

Condition-controlled loops use `while`:

```txt
while current < 10 {
    current += 1
}
```

`while let` repeats while a refutable pattern continues to match. The source is
evaluated before every iteration, and bindings are visible only in the loop
body:

```txt
while let candidate <- current.next {
    println(candidate.value)
    current := candidate
}
```

Conditions use the same freely ordered Boolean/`let` clause grammar as `if`.
They are evaluated from left to right, short-circuit on the first failure, and
later clauses may use bindings created by earlier `let` clauses:

```txt
while let candidate <- current.next && candidate.value == expected {
    current := candidate
}

while running && let candidate <- current.next && candidate.value == expected {
    current := candidate
}
```

Both refutable pattern forms are supported:

```txt
while let Some { value as item } = nextItem() {
    consume(item)
}

while let item <- nextItem() {
    consume(item)
}
```

An irrefutable `while let` pattern is rejected; use a Boolean `while` condition
or bind the value inside the body instead.

Infinite loop:

```txt
while true {
    if done {
        break
    }
}
```

Skipping to the next iteration:

```txt
for item <- [1, 2, 3] {
    if item == 2 {
        continue
    }
    println(item)
}
```

## `match`

Statement form:

```txt
match value {
    case SomeX { value as x } => {
        println(x)
    }
    case OptionX.NoneX => {
        println("none")
    }
}
```

Expression form:

```txt
result = match value {
    case SomeX { value as x } => x
    case OptionX.NoneX => 0
}
```

Guards are supported on cases with `if ... =>`:

```txt
result = match value {
    case SomeX { value as x } if x > 10 => x
    case SomeX { value: _ } => 10
    case OptionX.NoneX => 0
}
```

The depth-zero `=>` terminates the guard and starts the case body. Arrows inside
explicitly delimited nested expressions retain their normal meaning, so a guard
may call a lambda such as `values.exists(value => value > 0)`.

Case alternatives use `|` between patterns. The alternatives share one guard and
one body:

```txt
result = match value {
    case Size.Small | Size.Medium => "common"
    case Size.Large => "large"
}
```

Alternatives are tried from left to right. Once one alternative matches, the
shared guard is evaluated exactly once. If the guard is false, matching proceeds
to the next written case rather than trying another alternative from the same
case:

```txt
result = match pair {
    case (0, _) | (_, 0) if allowed() => "accepted"
    case _ => "rejected"
}
```

For `(0, 0)`, `allowed()` is called once even though both alternatives could
match.

Vector patterns can be used in `match` cases:

```txt
result = match values {
    case [] => "empty"
    case [only] => "single"
    case [first, second, ...rest] => "many"
}
```

`match` is always exhaustive; there is no separate partial-match form. Optional
results are ordinary values and must be expressed explicitly:

```txt
result Int? = match value {
    case SomeX { value as x } => Some(x)
    case OptionX.NoneX => None
}
```

To perform work for only one case, cover the remaining values explicitly:

```txt
match value {
    case SomeX { value as x } => println(x)
    case _ => ()
}
```

`match` always requires an explicit value and a block of cases: `match value { ... }`.

Every `match` branch must start with `case`.

Every case must have an explicit body after `=>`: an expression, `()` for Unit,
or a nonempty block. In an expression body, `{}` is empty construction and uses
the match expression's expected type. Without a concrete target, use `()` for
Unit.

```txt
match value {
    case Skip => ()
    case Empty => ()
    case Log { message } => {
        println(message)
    }
    case Other { message } => println(message)
}
```

Supported pattern families:

- wildcard: `_`
- whole-value alias: `_ as x`, `42 as value`, `None as none`,
  `Some(value) as some`, `[first, ...rest] as list`,
  `User { name } as user`
- literal/value patterns: `1`, `-1`, `-3.5`, `"hello"`, `true`
- case alternatives: `case A | B => ...`
- tuple patterns: `(x, y)`
- zero-payload cases and singletons: `None`, `Pending`, `Ready`
- optional payload shorthand: `^value`, equivalent to `Some(value)`
- unary named-data patterns: `Some(x)`, `Box(item)`
- named-field record patterns: `User { name }`, `Some { value }`
- unheaded record patterns for statically known values: `{ name, age }`
- type patterns: `item Worker`, `_ Other`

Type patterns use erased outer-type matching at runtime. For generic declared types, match on the outer name only:

```txt
match value {
    case _ Box => ...
    case _ Bag => ...
}
```

Generic arguments inside runtime type patterns are intentionally rejected for now, so use `_ Box` rather than `_ Box[Int]`.

Rules:

- declared-union exhaustiveness is checked
- every `match` must be exhaustive; use a wildcard case when all remaining values share one result
- a bare case-head identifier is a named zero-payload case or singleton, never
  a new local binding; use `_ as value` to bind an otherwise unrestricted value
- object alternatives use their bare name; `Alternative()` is invalid
- class, shape, primitive, and interface type tests use `_ Type` or `value Type`
- named-field patterns must select at least one field; `Type {}` is invalid
- negative numeric patterns are limited to `-` followed by an integer or float
  literal; arbitrary negated expressions are not patterns

### Record Patterns

Classes, named shapes, anonymous shapes, and payload alternatives use the same
name-based record pattern language:

```txt
case User { name }                    # bind field 'name' as local 'name'
case User { location as home }        # bind field 'location' as local 'home'
case User { age: 18 }                 # apply a literal pattern to field 'age'
case User { location: Location { city } }
case Ok { value }
case Err { error }
```

The forms compose recursively. `field: pattern` may contain a literal, tuple,
list, type, union-alternative, class, shape, or another record pattern.

Rules:

- fields match by name, never declaration order
- omitted fields are ignored; record patterns are partial by default
- at least one field must be selected
- only visible fields may be named
- constructor parameters do not participate in matching
- `field` binds a local with the same name
- `field as local` binds the field under another local name
- `field: pattern` applies a nested pattern to the field value
- `Type { fields } as value` also binds the complete matched value
- duplicate fields are invalid and field order is irrelevant
- a record pattern with only binding fields is irrefutable after its type or case test succeeds
- literals, nested refutable patterns, and runtime type tests make the containing pattern refutable
- generic runtime patterns use erased outer names; write `Box { value }`, not `Box[Int] { value }`

The same typed pattern is accepted everywhere patterns are used:

```txt
let User { name, age } = unknown else return Err(NotAUser)

if let User { name } = value {
    println(name)
}

for let User { name } <- users {
    println(name)
}

match value {
    case User { name } => println(name)
}
```

When the value's concrete class, shape, anonymous-shape, or declared-union type is
already known, omit the type head:

```txt
let { name, age } = user
let { city } = location
let { tag } = outcome

let { name, age: 18 } = user else {
    return Err("expected an 18-year-old user")
}

match profile {
    case { name, age: 18 } => println(name)
    case _ => ()
}

```

For declared unions, alternative-specific payload fields require the alternative head, for example
`Some { value }`, because the payload is not present on every alternative. Interface
values do not provide a concrete record layout and therefore require a type or
case pattern before record fields can be matched.

Parentheses provide unary named-data extraction only:

```txt
case Some(x) => ...
case Ok(value) => ...
case Err(error) => ...
case Box(item) => ...
case UserId(id) => ...
```

Because prefix `^` constructs `Some` in expressions, it also matches `Some`
in pattern positions:

```txt
let ^value = maybeValue else return

if let ^value = maybeValue {
    println(value)
}

match maybeValue {
    case ^value => println(value)
    case None => ()
}
```

`^pattern` is exactly shorthand for `Some(pattern)`. It accepts any nested
pattern, supports whole-pattern aliases, and composes one layer at a time, so
`^^value` matches `Some(Some(value))`. The source must have a compatible
`Option` type, and refutable `let` forms still require an `else` fallback.

`Type(pattern)` accepts one complete nested pattern, not merely a binding:

```txt
case Some(0) => ...
case Some(_) => ...
case Some(User { name }) => ...
case Box(Some(x)) => ...
case Ok(value Worker) => ...
case Some(x) as some => println(x, some.value)
```

Typed wildcard patterns compose at every nesting depth. Replacing a binding
name with `_` discards that binding without changing the available pattern
forms:

```txt
case _ Int => ...
case Some(_ Int) => ...
case (_ Int, _) => ...
case [_ Int, _ Str] => ...
case User { age: _ Int } => ...
```

Aliasing a unary union-alternative pattern retains that concrete alternative view,
so the complete alias exposes the matched fields while the nested pattern
continues to bind or test its payload.

At any pattern site, `as` may alias the complete matched value. This includes
literals and list patterns:

```txt
case 42 as value => println(value)
case "str" as text => println(text)
case [first, second, third] as list => println(first, list.size)
case [first, second, ...rest] as list => println(rest.size, list.size)
```

The alias is introduced only when the complete inner pattern succeeds.
The same alias form works in `let`, `if let`, `while let`, `for let`, and
`match`:

```txt
let User { name } as user = value else return

if let User { name } as user = value {
    println(name, user.name)
}

while let Some(value) as some = current {
    println(value, some.value)
}

for let User { name } as user <- users {
    println(name, user.name)
}
```

Every pattern position uses this same grammar, including tuple and record
patterns. The surrounding construct determines whether the pattern may fail:

```txt
let (left, right) as pair = source

let (Some(value), label) = pair else return

if let (Some(value), label) = pair {
    println(value, label)
}

for let (left, right) as pair <- pairs {
    println(left + right, pair)
}

values = for {
    let (left, right) as pair <- pairs
} yield left + right + pair[0]
```

`let pattern = value` requires an irrefutable pattern. Adding `else` handles
a refutable pattern. `if let` permits refutable branching, while `for let`
requires the pattern to be irrefutable for every generated element. `match`
permits refutable patterns and checks coverage where the matched type is
closed.

Field aliases remain available inside record patterns, for example
`let { location as home } = user`.

Conceptually, `Some(x)` is `Some { value as x }`, and `Some(User { name })`
is `Some { value: User { name } }`. The type or case must have exactly one
extractable field. Classes and shapes count visible data fields; union alternatives
count their payload fields; constructor parameters
never participate in matching.

Named data with multiple fields uses braces and fields are selected by name:

```txt
case User {
    name
    location as home
} => ...

case HttpError {
    status
    message
} => ...

case User(name, location) => ...       # invalid
case HttpError(status, message) => ... # invalid
```

Brace patterns must select at least one field. Omitted fields are ignored, but
an empty pattern such as `User {}` is redundant and rejected. Use a type
pattern when no fields are needed:

```txt
case _ User => ...       # match User and ignore the complete value
case user User => ...    # match User and bind the complete value
case User { name } => ...
case User { name } as user => ...
```

`User { name }` uses exactly the same runtime type test as `_ User`; braces add
field matching and do not select a different nominal or structural rule.
Interfaces and primitives support type patterns but not field patterns:

```txt
case value Str => ...
case reader Readable => ...
case _ Worker => ...
```

Use `_ as value` to bind a value without restricting its type:

```txt
case _ => ...
case _ as other => println(other)
```

Bare names match object alternatives or other singleton objects:

```txt
case None => ...
case Pending => ...
case Ready => ...
```

Zero-field classes and shapes use ordinary type patterns rather than empty
parentheses or braces:

```txt
case _ Marker => ...
case marker Marker => ...
```

`Type()` and `Type {}` are invalid patterns. Tuples remain positional because
tuples are positional data.

## Destructuring

Tuple destructuring:

```txt
let (left Int, right Str) = (5, "hello")
let (first, _, _) = tuple3
```

Tuple patterns must match the tuple's full arity. Use `_` for positions that
should be ignored; singleton tuple patterns such as `(first,)` are invalid.

Anonymous-shape and class destructuring use braces:

```txt
let { value Int, label Str } = { value: 7, label: "world" }
```

Class destructuring also uses braces:

```txt
let { left Int, right Str } = Box(9, "boxed")
```

Aliases use `as`:

```txt
let { location Str as loc, name as user } = user
```

Field pattern forms:

```txt
fieldName
fieldName Type
fieldName as localName
fieldName Type as localName
```

Brace destructuring for classes and anonymous shapes matches by field name.
That means:

- field order on the left does not matter
- partial destructuring is allowed by omission
- local aliases use `as`
- fields inaccessible at the destructuring site cannot be named
- `_` is not needed; just leave fields out

Examples:

```txt
let { name, location } = user
let { location, name } = user
let { location Str as loc, name as userName } = user
let { name } = user
```

Invalid because the names do not match fields:

```txt
let { usr, address } = user
```

Use the same name-based rule after binding an item in a `for` body or a
`for { ... } yield` `let` clause.

## Numeric semantics

`Int` is a signed 64-bit two's-complement integer. Integer arithmetic does not
silently change representation:

- `Int` versus `Int` arithmetic and ordering operate directly on 64-bit integers
- integer `+`, `-`, `*`, and unary `-` wrap modulo 2^64 on overflow
- integer `/` truncates toward zero and `%` has the sign of the dividend
- integer division or remainder by zero is a runtime error
- the overflowing `Int.MIN / -1` case wraps to `Int.MIN`; `Int.MIN % -1` is zero
- integer literals outside the signed 64-bit range are rejected

`Float` is an IEEE-754 binary64 value. Float arithmetic, division, remainder,
and ordering follow IEEE-754 behavior. Division by zero may produce positive or
negative infinity, `0.0 / 0.0` produces NaN, and NaN is unordered: every
ordering comparison with NaN is false. Float equality with NaN is false and
float inequality with NaN is true.

Mixed `Int`/`Float` arithmetic and ordering widen the `Int` operand to `Float`;
the result of mixed arithmetic is `Float`. That conversion can lose integer
precision. Equality remains stricter: `==` and `!=` require the same static
equality domain, so an `Int` and a `Float` are not directly comparable for
equality.

## Operators

Arithmetic:

- `+`
- `-`
- `*`
- `/`
- `%`

Comparison:

- `==`
- `!=`
- `===` (same concrete type and value equality)
- `!==` (different concrete type or value inequality)
- `<`
- `<=`
- `>`
- `>=`

Boolean:

- `!`
- `&&`
- `||`

Other operators / constructs:

- `.` for member access; `..` has no combined meaning and is rejected rather
  than being treated as one dot
- `is` and contextual `is not` for positive and negative runtime type checks
- `<-` for `for` iteration and success-case extraction in `if let` and `let ... else`
- `??` for extract-or-fallback through `Option`, `Result`, and `Either`
- `^` as shorthand for wrapping a value in `Some`
- `!` for unsafe extraction through `Option`, `Result`, and `Either`
- `fn(...) T` for function types
- `=>` for lambdas and by-name parameters
- `=>` for match cases
- `with` for interface implementation, generic bounds, and exact shape update
- `override ...source` for whole-source precedence in shape construction
- `when` for generic bound and equality conditions
- `|` between type alternatives in a union and between alternatives in a match case
- `:` inside map types, map literals, and construction field lists

Expression precedence, from highest to lowest:

| Level | Forms | Associativity |
| --- | --- | --- |
| Primary | literals, groups, blocks, value-producing control flow | n/a |
| Postfix | calls, member access, indexing, Vector slicing, `!` | left |
| Unary | `-`, `!`, `^`, `try` | right |
| Multiplicative | `*`, `/`, `%` | left |
| Additive | `+`, `-` | left |
| Shape update | `with` | left |
| Comparison | `<`, `<=`, `>`, `>=` | non-associative |
| Type test | `is`, `is not` | non-associative |
| Equality | `==`, `!=`, `===`, `!==` | non-associative |
| Boolean AND | `&&` | left |
| Boolean OR | `||` | left |
| Extract or fallback | `??` | right |

Comparison, equality, strict equality, and runtime type-test operators cannot
be chained without parentheses:

```txt
a < b < c       # invalid
a == b == c     # invalid

a < b && b < c  # valid
(a == b) == c   # valid when the operand types permit it
```

Parenthesized subexpressions start a new comparison level, making unusual but
intentional Boolean comparisons explicit.

Value-producing `if / else`, `match`, and `for ... yield` are
primary expressions. After the control-flow expression closes, surrounding
postfix and infix parsing continues normally, and these forms may also appear as
operator operands:

```txt
value = if flag { 10 } else { 20 } - 1

value = match flag {
    case true => 10
    case false => 20
} - 1

value = 1 + if flag { 2 } else { 3 }
```

The shape-update level means:

```txt
a + b with p       # (a + b) with p
a with p + q       # a with (p + q)
a with p == q      # (a with p) == q
a with b with c    # (a with b) with c
```

Use parentheses to override these groupings, including when a patch is another
shape-update expression: `a with (b with c)`.

Examples:

```txt
counter is Counter
for item <- items {
}
fn(Int) Str
SomeX(x) => x
class Box[T] with Named
pair = ("a", 1)
name = maybeName ?? "unknown"
```

Shape copy, extension, and collision-protected merge:

```txt
copy = { ...user }
extended = {
    ...user
    location: "New York"
}
merged = {
    ...named
    ...located
}
```

Operator declarations use symbolic `def` forms on interfaces and classes. Shared
declared-union behavior belongs in an extension block on the union:

```txt
def +(other Vec) Vec = Vec(this[0] + other[0], this[1] + other[1])
def -() Vec = Vec(-this[0], -this[1])
def [](index Int) Int = this.items[index]
```

Current operator overloading constraints:

- Allowed to overload:
  - arithmetic: `+`, `-`, `*`, `/`, `%`
  - unary: unary `-`
  - indexing: `[]`
- Not allowed to overload:
  - logical operators: `&&`, `||`, `!`
  - equality operators: `==`, `!=`, `===`, `!==`
  - symbolic collection/custom forms: `:+`, `:-`, `++`, `--`, `::`
- Comparison operators use intrinsic numeric and string ordering, or `Ordered[T]` for user-defined values. `Ordered[T]` declares `compare(other T) Int`.
- `==`, `!=`, `===`, and `!==` use the applicable `Eq[T]` contract and cannot be overloaded independently; `===` and `!==` additionally test concrete type.
- Reference identity is expressed by equality between opaque `referenceId` values; `ReferenceId` equality and hashing cannot be overloaded.
- Standard collections do not define symbolic operators like `:+`, `:-`, `++`, or `--`; collection APIs should prefer searchable method names.
- `:` is brace-entry syntax only, not an overloadable operator.
- The spellings `:+`, `:-`, `++`, `--`, and `::` are removed from the language surface and currently produce `unsupported_operator` lexer diagnostics.

Newline continuation:

- Ordinary expressions are no longer broadly newline-insensitive.
- A newline continues the current expression only when the previous line clearly ends in a continuation form, except postfix chains may continue when the next line starts with `.`.
- Continuation tokens:
  - binary operators: `+`, `-`, `*`, `/`, `%`, `&&`, `||`, `==`, `!=`, `===`, `!==`, `<`, `<=`, `>`, `>=`
  - extraction/fallback operators: `??`
  - exact shape update introducer: `with`
  - unary prefixes: unary `-`, `!`, `^`, `try`
  - runtime type check keywords: `is`, and contextual `not` after `is`
  - match arrow: `=>`
  - separators / chaining markers: `,`, `.`
- Delimited forms allow layout after opening delimiters and after commas, but they do not make leading binary/update operators valid by themselves.
- Operators that require a right-hand expression may start that expression on
  the same line or the next indented line: `=`, `:=`, `+=`, `-=`, `*=`, `/=`,
  `%=`, and `<-`.
- Named function and method declarations require `def` and have three accepted body forms:
  - `def name(...) { ... }` for block bodies, with implicit `Unit` when the return type is omitted
  - `def name(...) = { ... }` for block bodies whose return type is inferred when omitted
  - `def name(...) = expr` for expression bodies whose return type is inferred when omitted
- Constructors are the only callable declarations without `def`; they begin with `new` and use the constructor body forms documented above.
- Inline-body introducers such as `else` and `yield` may take a same-line body without braces; if that body moves to the next line, a `{ ... }` block is required.
- So this is valid:

```txt
a =
    1 + 2

var total = 0
total +=
    calculateTotal()

let item <-
    findItem()
else return
```

- while this stays valid:

```txt
a = 1 +
    2

updated = user with
    { age: 42 }
```

- but this is invalid:

```txt
a = 1
    + 2
```

- and this also stays valid:

```txt
def value() Int =
    1 + 2

if flag {
    return 1
}
```

- Dot chaining allows both trailing-dot and line-leading postfix styles:

```txt
size = "hello".
    size

size = "hello"
    .size
```

## Visibility

Supported today:

- `internal` on top-level declarations, immutable bindings, fields, methods, and constructors
- `private` on top-level `def`
- `private` on top-level immutable bindings
- `private` on top-level `interface`
- `private` on top-level `class` / `shape` / `object` / `type`
- `private` on fields
- `private` on methods
- `private` on constructors

Default visibility is public. There is no `public` keyword.
Use `internal` for declarations and members shared by files with the same
`module` name. Other modules cannot import an internal top-level declaration or
access an internal member.

Use `private` for declarations local to their source file and for members local
to their declaring type. A class method may access private members on another
instance of the same class; extension methods cannot access private members.

Shape and annotation fields are always public structural data, so they cannot
be `internal` or `private`.
Top-level mutable bindings are not allowed; mutable module state must live inside
named objects, class instances, or function locals.

## Notes

This file is meant to describe the current surface syntax.

Ideas that are still under discussion belong in `features.md`, not here.
