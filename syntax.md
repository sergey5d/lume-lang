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
- `Ordering[T]`
- `Printer`
- `OS`

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
- `value.sameValue(other)` performs strict dynamic equality by comparing runtime type witnesses before value equality

`Any` does not support ordinary equality because it erases the static equality
domain. Narrow it before using `==`, or deliberately use `sameValue`:

```txt
unknown Any = Point(1, 2)
point = Point(1, 2)

unknown == point          # error
unknown.sameValue(point)  # true: same runtime type and equal value
```

Reference identity is separate from value equality:

```txt
alias = value
copy = Box(value.field)

value === alias # true: both names reference the same instance
value === copy  # false: copy is a different instance
value !== copy  # true
```

`===` and `!==` do not call `equals`. They accept compatible class, object, or
concrete collection reference types. Primitives, shapes, enums, interfaces,
`Any`, and lifted wrappers such as `Option[T]` are not identity operands; unwrap
or narrow to a concrete reference type first.

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
a.size()              # allowed
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

Function types use `fn` directly before a parenthesized parameter-type list,
followed by the return type without an arrow. Write `fn(Int) Int`, not
`fn(Int) => Int`, `(Int) => Int`, `Int => Int`, or `fn (Int) Int`.
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

Common metadata operations:

```txt
println(typeOf[User].name() !!)
println(typeOf[User].kind())

classType ClassType[User] = typeOf[User].asClass() !!
fields = classType.fields()
let Some { value as nameField } = classType.field("name") else panic("expected name field")
println(nameField.fieldType().name() !!)
println(nameField.isHidden())

enumType EnumType[Status] = typeOf[Status].asEnum() !!
let Some { value as pendingCase } = enumType.case("Pending") else panic("expected Pending case")
println(pendingCase.name())
constructedCase Result[Any, ReflectionError] = pendingCase.construct()
```

Safe reflective invocation uses `Result` values:

```txt
constructed Result[User, ReflectionError] = classType.construct("Ada", 42)

user User = constructed !!
nameValue Result[Any, ReflectionError] = nameField.get(user)

let Some { value as greetMethod } = classType.method("greet") else panic("expected greet method")
greeting Result[Any, ReflectionError] = greetMethod.call(user)
```

`Method.invoke(receiver, args...)` is also available as the direct invocation
form and returns `Any`; it may panic if the method is not invokable or the call
fails. Prefer `call` when failures should stay in the value model.

Rules:

- `typeOf[T]` is a built-in type metadata operator, not an index operation
- `runtimeType` is available as a read-only synthetic field on values
- `TypeKind` includes `Class`, `Shape`, `Enum`, `Interface`, `Object`, `Annotation`, `Primitive`, `Tuple`, `Function`, and `AnonymousShape`
- field, method, parameter, and enum-case metadata are runtime values with methods such as `name()`, `fieldType()`, `isHidden()`, `params()`, and `returnType()`
- annotation lookup is typed and reified: use `metadata.hasAnnotation[Route]()` and `metadata.annotation[Route]()`
- reflective construction is supported for class and named shape metadata through `construct(args...)`
- reflective enum case construction is supported through `EnumCase.construct(args...)`
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
- `Str.size()` returns the string length as `Int`

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
```

`OS.stdout` and `OS.stderr` implement `Printer`.

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

enum RouteVisibility {
    case External
    case Internal
}

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
- enum cases, such as `RouteVisibility.External`
- arithmetic, comparison, boolean, and string-concatenation expressions whose operands are also annotation-safe

Calls, constructors, indexing, mutable object fields, ordinary instance field reads, `try`, `for ... yield`, `match`, `if`, lambdas, and blocks are rejected in annotation arguments. Top-level mutable bindings are not allowed at all, so they are rejected before annotation argument checking.

Supported annotation targets:

- top-level `def`, `annotation`, `interface`, `class`, `shape`, `object`, `enum`
- fields
- methods
- interface methods
- enum cases

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
- `enum`
- `ext TypeName`
- `name Type = expr`
- `hidden def`
- `hidden name Type = expr`
- `hidden annotation`
- `hidden interface`
- `hidden class`
- `hidden shape`
- `hidden object`
- `hidden enum`

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

hidden shape InternalPoint {
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

enum OptionX[T] {
    case NoneX
    case SomeX {
        value T
    }
}
```

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

Top-level immutable bindings are also supported:

```txt
seed Int = 1
hidden internalSeed Int = 0
```

Top-level mutable bindings are not allowed. Mutable module state must live
inside a named `object`, class instance, or function local.

Fields without initializers are only valid in class-like field declarations:

```txt
class Box {
    hidden var cached Int
    hidden label Str
}
```

`hidden` fields in classes and named objects may infer their type from an initializer:

```txt
class Box {
    hidden count = 0
    hidden var hits = 0
}

object Greeter {
    hidden hello = "Hello"
}
```

Visible class fields, shape fields, enum fields, and object fields still require explicit field types.

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

`base with patch` updates existing visible fields. `base` must be a class, named
shape, or anonymous shape. `patch` must be a statically known shape-like value.
Every visible field in `patch` must already exist on `base`, and each patch
field type must be assignable to the corresponding base field type. The result
keeps the same class/shape view as `base`. Hidden fields are not updated through
`with`, and the source value is not mutated.

## Construction

Braces are for construction fields:

```txt
user = { name: "Ada", age: 10 }
user User = User { name: "Ada", age: 10 }
```

Parentheses are for positional construction and calls:

```txt
user User = User("Ada", 10)
maybe = Some(5)
```

Braces are also the field construction form for enum payload cases:

```txt
maybe = Some { value: 5 }
```

Zero-payload enum cases are bare values, not calls:

```txt
none = None
```

Anonymous shapes use either bare braces or the explicit `shape` prefix for
construction fields. The forms are equivalent:

```txt
user = {
    name: "Ada"
    age: 10
}

explicitUser = shape {
    name: "Ada"
    age: 10
}
```

The explicit form is useful where several brace forms appear together. It also
provides the unambiguous empty anonymous shape `shape {}`; bare `{}` remains an
empty block expression.

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
```

Anonymous shape positional construction uses `shape(...)`. It is contextual:
the expected type must be an anonymous shape type, and values map to fields in
the written field order:

```txt
user { name Str, age Int } = shape("Ada", 10)

def makeUser() { name Str, age Int } {
    return shape("Ada", 10)
}

describe(shape("Cara", 14))
```

Without an expected anonymous-shape type, field names are unknown, so
`shape(...)` is rejected:

```txt
user = shape("Ada", 10)      # invalid
```

An overloaded call cannot provide the required expected type unless the overload
set selects one anonymous-shape parameter unambiguously. If multiple overloads
could accept the same `shape(...)` values with different field names, bind an
intermediate anonymous shape first.

Tuples do not construct shapes or classes. Classes must name their constructor
target:

```txt
user User = User { name: "Ada", age: 10 }
person Person = Person("Ben", 12, "NYC")
profile MixedProfile = MixedProfile {
    name: "Liam"
    age: 8
}
tail HiddenTail = HiddenTail("Ada", 4)
settings Settings = Settings {}
```

Named shapes are data-only structural field views:
- fields are always visible and read-only
- fields are declared in the `shape` body
- methods are declared directly in the `shape` body after fields
- custom `new` constructors are not allowed
- shapes may declare interface bounds with `shape Name with Interface`
- brace field construction uses `ShapeName { field: value }`
- positional construction uses `ShapeName(...)`

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

Construction rules:

General construction rules:

- a constructor declaration lists the inputs accepted by construction
- constructor parentheses accept positional arguments only; use braces for construction fields
- function and method calls may still use named arguments in parentheses
- `Type { value }` is not valid; use `Type(value)` only when the type supports positional construction
- anonymous shapes use `{ field: value }` or `shape { field: value }` for field construction and `shape(...)` for contextual positional construction
- runtime-backed collection classes such as `Vector`, `Map`, `Array`, `LinkedList`, and `Set` use normal class construction; `Range(...)` is a stdlib factory
- `Type { ... }` resolves through the available explicit `new(...)` declaration or implicit field-construction inputs
- `Type(...)` resolves through explicit class `new(...)`, implicit visible-field construction, named shape positional construction, or an intrinsic collection form
- class construction is nominal and constructor-gated; shape construction is structural
- tuple values cannot construct classes or shapes; write `shape(...)`, `Point(...)`, `User(...)`, or construction fields
- nested inner constructions must still name the target class explicitly, often by binding the inner value first, for example `leader = Person { name: "Ada", age: 10 }` and then `owner = Team { leader: leader }`

Explicit constructor rules:

- `new(field Type, other Type = default) { ... }` declares explicit constructor inputs with required and defaulted parameters
- constructor parameters do not have to be class fields; they are inputs to the constructor body
- `Type { field: value, other: value }` matches explicit constructor inputs by parameter name
- `Type(value, otherValue)` fills the same explicit constructor inputs by declaration order
- constructor parameters may be declared in the author's preferred order; defaults do not have to trail required parameters
- named construction may omit any constructor parameter that has a default
- positional construction fills a prefix of constructor parameters in declaration order
- positional construction may omit only a trailing suffix whose parameters all have defaults
- positional construction never skips a defaulted parameter to reach a later required parameter
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

# invalid: fills `body`, then leaves required `title` unset
bad Article = Article("Intro")
```

Implicit field construction rules:

- if a class has no explicit `new`, the compiler synthesizes constructor inputs from visible fields
- construction braces check the synthesized visible-field shape
- visible fields without initializers are required
- visible fields with initializers are optional
- hidden fields are excluded from the synthesized constructor inputs
- hidden fields without initializers suppress implicit field constructors; define `new` to initialize them
- `Type {}` works when the synthesized field-construction shape has no required fields
- positional construction follows declared visible-field order
- positional construction may omit only a trailing suffix of visible fields that all have initializers
- positional construction never skips a defaulted visible field to reach a later required visible field
- positional construction is rejected when a hidden initialized field appears before a later visible field
- mutable vs immutable field differences do not matter for structural shape matching
- named class values do not structurally convert to other named class values

## Brace Disambiguation

Braces carry several meanings. The parser chooses by the tokens before and inside the braces:

```txt
{ field: value }                 # anonymous shape literal
{ field Type: value }            # typed anonymous shape literal
shape { field: value }           # explicit anonymous shape literal
shape {}                         # explicit empty anonymous shape literal
{ expr }                         # block expression
Type { field: value }            # brace field construction or enum field payload
call { x => ... }                # trailing lambda
object { field Type = value; def method() Type = value } # anonymous object
object with Interface, Other { def method() Type = value } # anonymous interface implementation
new(field Type)                  # constructor declaration
shape(value, other)              # contextual anonymous-shape positional construction
```

Single-expression braces such as `{ value }` are block expressions, not anonymous shapes. To construct an anonymous shape, use construction fields with `:`. Bare `{}` is an empty block; use `shape {}` for an empty anonymous shape.

Shape conversion rules:
- field names and field types must match at compile time
- extra fields are allowed when passing a value to a narrower shape
- missing fields are rejected
- defaults are not part of the shape syntax
- `shape(...)` may construct anonymous shapes only when the expected type is an anonymous shape
- `shape(...)` values map to anonymous-shape fields in written field order
- `shape(...)` argument count must exactly match the anonymous-shape field count
- shape-to-shape assignment is structural by field names and field types
- class-to-shape is allowed through visible fields
- shape-to-interface follows the shape's explicit `with Interface` bounds
- class-to-interface-through-shape is not automatic; assign the class value to an explicit shape view first
- hidden class fields are not visible to shape conversion
- shape-to-class is not implicit; use a class constructor
- tuple-to-shape and tuple-to-class are not allowed; use `shape(...)`, named shape construction, or class constructors
- ordinary calls may still accept named anonymous shapes in parentheses, for example `describe({ name: "Cara", age: 14 })`
- `shape { ... }` is exactly the explicit spelling of an anonymous shape literal and accepts the same typed fields and spreads as bare field braces
- construction fields inside braces use `field: value`
- construction fields may carry an explicit initializer type as `field Type: value`
- single-expression braces like `{ value }` are still block expressions, not anonymous shapes

Shape equality is structural across shape declarations:

- comparing two shapes with `==`, `!=`, or `equals(...)` requires the same complete set of field names and normalized field types
- field declaration order may differ
- the right operand is converted to the left operand's shape by field name, then ordinary value equality is applied
- unlike shape assignment, equality does not ignore extra fields
- width-compatible shapes must first be explicitly projected through shape assignment
- generated Java shape records define field-based `equals` and matching `hashCode` methods; hash inputs are ordered by field name so declaration order does not change the hash

```txt
shape Point { x Int, y Int }
shape Position { y Int, x Int }
shape Point3D { x Int, y Int, z Int }

Point(1, 2) == Position(2, 1) # true: fields match by name
Point(1, 2) == Point3D(1, 2, 3) # error: schemas differ

point2d Point = Point3D(1, 2, 3)
point2d == Point(1, 2) # true after explicit projection
```

Other equality domains are nominal. The operands must have the same normalized
type. Classes may use equality only when they explicitly implement
`Eq[ClassName]`; interface values require a compatible explicit `Eq` contract.
Different classes are not directly comparable. `sameValue` returns `false`
instead of producing a static error when its runtime equality domains differ.

| Operands | `==` / `!=` | `sameValue` |
| --- | --- | --- |
| same shape schema | field equality | descriptor match, then field equality |
| different shape names, same schema | field equality by name | `false`; named descriptors differ |
| width-compatible shape schemas | error; project explicitly first | `false` |
| class and shape | error; project explicitly before erasure | `false` |
| same class with `Eq[Class]` | declared class equality | concrete descriptor, then class equality |
| different classes | error | `false` |
| `Any` and a typed value | error | descriptor comparison |
| `Any` and `Any` | error | descriptor comparison |
| interface values | requires an explicit compatible `Eq` domain | concrete descriptor comparison |

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

- primitives, enums, and object values are intrinsically hashable
- nested shapes are hashable when their own fields are recursively hashable
- a class is hashable only when it explicitly implements `Hashed[ClassName]`, including `equals` and `hash`
- a type parameter is hashable only when it has a `Hashed[T]` bound
- interfaces, functions, tuples, `Any`, and arbitrary classes do not implicitly satisfy `Hashed`

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

point Point = Point(1, 2)           # named shape positional construction
anon { x Int, y Int } = shape(1, 2) # anonymous shape positional construction
fromClass Point = Pixel { x: 1, y: 2 } # class -> shape
named Point = { x: 1, y: 2 }        # anonymous shape -> named shape

user User = ("Ada", 10)             # invalid: tuple -> class
point Point = (1, 2)                # invalid: tuple -> named shape
named Point = shape(1, 2)           # invalid: use Point(1, 2)
anon = shape(1, 2)                  # invalid: expected shape fields are unknown
user User = { name: "Ada", age: 10 } # invalid: shape -> class
```

Typed anonymous shape fields:

```txt
user = {
    name Str: "Ada"
    age Int: 42
}
```

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
    typeOf[A].name() !!

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

Explicit generic application has priority over indexing when it is immediately
called. Any expression of the form `callee[...](...)` is parsed as a generic
call. To call a function value returned from indexing, group the indexed
expression:

```txt
metadata[User]()       # explicit generic call
entries["a"]           # indexing
(handlers[key])()      # call an indexed function value
handlers[key]()        # invalid: parsed as generic call syntax
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
- Parameters with defaults must form a trailing suffix.
- A variadic parameter may have a default vector value, written after `vararg`.

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

- Use by-name parameters only for conditional-value APIs such as `assert`, `debug`, `getOr`, and `orElse`.
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
value = maybe.getOr(expensiveDefault())
result = maybe.toResult(makeError())
next = result.orElse(recover())
mapped = maybe.map(value => value + 1)
leftMapped = either.mapLeft(error => error.toStr())
```

Classes, shapes, enums, and named objects declare methods directly in their
declaration bodies, after storage fields and constructors:

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
- extension targets may be classes, shapes, enums, interfaces, or built-in primitive types such as `Int`, `Float`, `Bool`, `Str`, and `Rune`
- extension targets cannot be named objects, annotations, or enum cases
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
- shape, enum, enum case, object, annotation, and interface declarations cannot define custom `new` constructors
- constructor parameters use `name Type`, with optional defaults such as `age Int = 0`
- `Type { field: value }` constructs by matching constructor parameters by field name
- `Type(value)` constructs by filling constructor parameters positionally by declaration order
- named construction may omit any constructor parameter with a default
- positional construction fills a prefix of constructor parameters and may omit only trailing parameters that all have defaults
- positional construction never skips a defaulted parameter to reach a later required parameter
- constructor parameters may end with one variadic vector parameter such as `items [Str] vararg`
- `hidden new(...) { body }` declares a private constructor
- each explicit class constructor must initialize every field that does not have a field initializer, or delegate to another constructor
- `this(...)` inside a constructor delegates positionally to another constructor of the same class
- `this { field: value }` inside a constructor delegates with construction fields to another constructor of the same class
- delegating constructors use expression bodies, for example `new(label Str) = this { name: label }`
- direct and indirect constructor-delegation cycles are rejected
- `new(...)` only declares constructors; it is not a constructor-delegation call
- class call sites use braces for construction fields, for example `Person { name: "Ada", age: 10 }`
- class call sites use parentheses for positional arguments, for example `Person("Ada", 10)`
- `this` is the instance receiver
- instance fields on classes, enums, and named objects may be accessed bare when they are not shadowed
- use `this.field` when a parameter/local shadows a field, for example `this.age`
- member order is storage first, constructors next, methods last
- class, shape, enum, and object bodies list storage fields before behavior
- enum cases count as enum storage and must appear before enum methods
- a class body may declare constructors after its fields and before its methods

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

Fields still win over methods when names collide. A member field whose value is
already a function is passed as that function value, not eta-expanded as a
method reference.

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

Multiple statements require an explicit block. This keeps lambda scope
brace-delimited:

```txt
# invalid
mapper = item =>
    next = item + 1
    next * 2

# valid
mapper = item => {
    next = item + 1
    next * 2
}
```

Trailing lambda call syntax is also allowed when passing a lambda as an argument. The trailing brace body must contain an explicit lambda head with `=>`, and that lambda head must start on the same line as the opening `{`:

```txt
items.map { x => x + 1 }

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

Trailing brace call syntax on non-constructor calls is only for lambda arguments.
Constructor braces fill constructor inputs by field name, so enum named
payloads use braces and enum positional payloads use parentheses:

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

The same idea applies to `partial match`:

```txt
options.map(value => partial match value {
    case SomeX { value as x } => x + 1
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
- named function and method block bodies may be written as `def name(...) { ... }` or `def name(...) = { ... }`
- if you want a block value, the last statement must be value-producing
- value-producing tail forms include ordinary expressions, `if / else`, `match`, and `for ... yield`
- blocks can nest arbitrarily

## Classes, Shapes, Objects, Interfaces, Enums

Class:

```txt
class Box[T] with Named {
    value T
    def label() Str = "box"
}
```

When a class, shape, enum, or named object implements an interface method inside
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
- fields appear before methods
- anonymous objects cannot declare `new` constructors
- fields and methods are statically typed and use ordinary member access
- `this.field` and unqualified `field` are both available inside methods

To create an anonymous interface implementation, use `object with`:

```txt
greeter Greeter = object with Greeter {
    def greet() Str = "hello"
}
```

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

Anonymous implementations always start with `object with`. Repeating the
keyword, such as `object with Readable with Writable`, is invalid; write
`object with Readable, Writable` instead.

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

Enums:

```txt
enum Color {
    code Str

    case Red {
        code = "red"
    }

    def isWarm() Bool = code == "red"
}
```

```txt
enum OptionX[T] {
    case NoneX
    case SomeX {
        value T
    }
}
```

Enum cases are data-only:

- cases may declare payload fields
- cases may assign shared enum fields
- cases may not declare methods
- cases may not declare custom constructors
- zero-payload cases are values and are written without call syntax, for example `None`
- payload cases use positional constructor syntax, for example `Some(value)`
- payload cases may also use construction fields in braces, for example `Some { value: value }`
- payload and shared fields with defaults may be omitted from enum case constructors
- `None()`-style calls for zero-payload cases are invalid

Behavior for enums belongs directly in the enum declaration body. Case-specific
behavior should be expressed with `match`.

## Calls

Normal call:

```txt
add(1, 2)
```

Named arguments:

```txt
format(prefix = "item", value = 5)
```

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

`at`, `first()`, `last()`, `removeFirst()`, and `removeLast()` are safe and
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

Vectors expose `at`, `setAt`, `insertAt`, and `removeAt` with the same safe
return types as LinkedList. Vector and Array bracket access remains available
as the explicit unsafe alternative.

Map construction:

```txt
entries [Str : Int] = ["a": 1, "b": 2]
empty [Str : Int] = []
value Option[Int] = entries["a"]

defaults [Str : Int] = ["port": 80, "secure": 0]
overrides [Str : Int] = ["port": 443]
copy = [...defaults]
merged = [...defaults, "retries": 3, ...overrides]
```

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

Tuples always contain at least two elements. Singleton tuple values, types, and
patterns are all invalid:

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
LIFO order when the enclosing function, method, or lambda returns.

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

Runtime type tests use `is`:

```txt
if value is Str {
    println(value.size())
}
```

`is` is non-associative. A type test has the grammar
`comparison ["is" type]`, so chained tests are rejected:

```txt
value is Str                 # valid
value is Str is Any          # invalid
(value is Str) && otherCheck # valid
```

Inside the successful branch, an immutable local binding or parameter tested
directly by name is narrowed to the tested type. Parentheses and a leading `!`
are recognized. When the opposite branch exits, the positive narrowing remains
available afterward:

```txt
def size(value Any) Int {
    if !(value is Str) {
        return 0
    }

    value.size()
}
```

This narrowing is intentionally local and conservative:

- mutable bindings are not narrowed, because another read may observe a different value
- by-name parameters, member reads, indexes, and arbitrary expressions are not narrowed
- `&&` / `||` conditions do not currently combine narrowing facts
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

`let ... else` is the refutable binding form with an explicit fallback path:

```txt
let Some { value as item } = maybeValue else {
    return Err("missing")
}
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

let [...all] = values
```

Vector pattern rules:

- `[a, b]` matches exactly two elements.
- `[]` matches an empty vector.
- `[a, ...rest]` matches one or more elements and binds `rest` as `[T]`.
- `[...rest]` matches any vector and binds a shallow vector tail copy as `[T]`.
- Only one `...rest` is allowed, and it must be last.
- `..._` ignores the remaining elements.
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

`let ... else` is statement-oriented:
- the pattern is matched against the right-hand value
- if the match succeeds, bindings remain visible after the statement
- if the match fails, the `else` block is evaluated and must exit the current control-flow path, typically with `return`, `break`, `continue`, or a call whose return type is `Never`

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
assert(split.size() == 3)
assert(split.size() == 3, "split must have 3 parts")
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

Unsafe extraction uses postfix `!!`:

```txt
value = wrapped !!
```

It extracts the success value from `Option[T]`, `Result[T, E]`, or
`Either[L, T]` and panics when the value is empty / error / left. Use it only
when failure is a programming error; prefer `try`, `??`, or `let ... else` for
recoverable control flow.

`!!` is a normal postfix operator. Whitespace before it is optional, and calls,
indexing, member access, and further extraction may follow it directly:

```txt
item = values[index]!!
item = values[index] !!
name = wrapped!!.name
result = callback!!()
first = wrappedVector!!.at(0)!!
entry = wrappedMap!!["key"]!!
nestedValue = nested!!!!
```

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

And grouped clauses can use `<-` too:

```txt
if let {
    left <- maybeLeft
    right <- maybeRight
} {
    println(left + right)
}
```

And `if let` conditions can be chained with `&&` so later clauses can use
earlier bindings:

```txt
if let Some { value as left } = maybeLeft && let Ok { value as right } = compute() && right > left {
    println(left + right)
}
```

Only `&&` joins are supported in this form.

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
result = items.filterMap(item => partial match item {
    case Some { value } => value
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

Conditions may be chained with `&&`. They are evaluated from left to right,
short-circuit on the first failure, and later clauses may use bindings created
by earlier `let` clauses:

```txt
while let candidate <- current.next && candidate.value == expected {
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

Case alternatives use `|` between patterns. The alternatives share one guard and
one body:

```txt
result = match value {
    case Size.Small | Size.Medium => "common"
    case Size.Large => "large"
}
```

Vector patterns can be used in `match` cases:

```txt
result = match values {
    case [] => "empty"
    case [only] => "single"
    case [first, second, ...rest] => "many"
}
```

Partial match expression form:

```txt
result Option[Int] = partial match value {
    case SomeX { value as x } => x
}
```

Partial match statement form executes the matching case if one exists and does
nothing when no case matches:

```txt
partial match value {
    case SomeX { value as x } => println(x)
}
```

Partial mapped through an explicit lambda:

```txt
values.map(value => partial match value {
    case SomeX { value as x } => x + 1
})
```

`match` and `partial match` always require an explicit value and a block of cases.
`match value { ... }`.

Every `match` and `partial match` branch must start with `case`.

Every case must have an explicit body after `=>`: an expression, `()` for Unit, or a block such as `{}`.

```txt
match value {
    case Skip => ()
    case Empty => {}
    case Log { message } => {
        println(message)
    }
    case Other { message } => println(message)
}
```

If no case matches, `partial match` returns `None`.

Supported pattern families:

- wildcard: `_`
- binding pattern: `x`
- literal/value patterns: `1`, `"hello"`, `true`
- case alternatives: `case A | B => ...`
- tuple patterns: `(x, y)`
- named-field record patterns: `User { name }`, `Some { value }`
- unheaded record patterns for statically known values: `{ name, age }`
- type patterns: `item Worker`, `_ Other`

Type patterns use erased outer-type matching at runtime. For generic declared types, match on the outer name only:

```txt
match value {
    _ Box => ...
    _ Bag => ...
}
```

Generic arguments inside runtime type patterns are intentionally rejected for now, so use `_ Box` rather than `_ Box[Int]`.

Rules:

- enum exhaustiveness is checked
- expression `partial match` skips exhaustiveness checking and wraps the result in `Option[...]`
- statement `partial match` skips exhaustiveness checking and does nothing when no case matches
- bare zero-payload enum cases should still be written in qualified form when needed, for example `MaybeInt.NoneX`

### Record Patterns

Classes, named shapes, anonymous shapes, and enum cases use the same
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
list, type, enum-case, class, shape, or another record pattern.

Rules:

- fields match by name, never declaration order
- omitted fields are ignored; record patterns are partial by default
- only visible fields may be named
- constructor parameters do not participate in matching
- `field` binds a local with the same name
- `field as local` binds the field under another local name
- `field: pattern` applies a nested pattern to the field value
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

When the value's type is already known, omit the type/case head:

```txt
let { name, age } = user

match profile {
    case { name, age: 18 } => println(name)
    case _ => ()
}
```

Tuples alone use positional parentheses. Named-field class and shape patterns
use braces rather than `User(name, age)` or `Point(x, y)`. Zero-payload enum
cases remain bare.

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
- hidden fields cannot be named
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
- `===` (reference identity)
- `!==` (reference non-identity)
- `<`
- `<=`
- `>`
- `>=`

Boolean:

- `!`
- `&&`
- `||`

Other operators / constructs:

- `is` for runtime type checks
- `<-` for `for` iteration and success-case extraction in `if let` and `let ... else`
- `??` for extract-or-fallback through `Option`, `Result`, and `Either`
- `!!` for unsafe extraction through `Option`, `Result`, and `Either`
- `fn(...) T` for function types
- `=>` for lambdas and by-name parameters
- `=>` for match cases
- `with` for interface implementation, generic bounds, and exact shape update
- `override ...source` for whole-source precedence in shape construction
- `when` for generic bound and equality conditions
- `:` inside map types, map literals, and construction field lists

Expression precedence, from highest to lowest:

| Level | Forms | Associativity |
| --- | --- | --- |
| Postfix | calls, member access, indexing, `!!` | left |
| Unary | `-`, `!`, `try` | right |
| Multiplicative | `*`, `/`, `%` | left |
| Additive | `+`, `-` | left |
| Shape update | `with` | left |
| Comparison | `<`, `<=`, `>`, `>=` | left |
| Type test | `is` | non-associative |
| Equality | `==`, `!=`, `===`, `!==` | left |
| Boolean AND | `&&` | left |
| Boolean OR | `||` | left |
| Extract or fallback | `??` | right |

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

Operator declarations use symbolic `def` forms on interfaces, classes, and enums:

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
- Comparison operators are intended to work through `Ordering[T]` rather than custom operator declarations.
- Value equality is intended to work through `Eq[T]`; reference identity is intrinsic and cannot be overloaded.
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
  - unary prefixes: unary `-`, `!`, `try`
  - runtime type check keyword: `is`
  - match arrow: `=>`
  - separators / chaining markers: `,`, `.`
- Delimited forms allow layout after opening delimiters and after commas, but they do not make leading binary/update operators valid by themselves.
- Binding/function/method `=` may start its expression on the same line or the next indented line.
- Named function and method declarations require `def` and have three accepted body forms:
  - `def name(...) { ... }` for block bodies
  - `def name(...) = { ... }` for block bodies
  - `def name(...) = expr` for expression bodies
- Constructors are the only callable declarations without `def`; they begin with `new` and use the constructor body forms documented above.
- Inline-body introducers such as `else` and `yield` may take a same-line body without braces; if that body moves to the next line, a `{ ... }` block is required.
- So this is valid:

```txt
a =
    1 + 2
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
    size()

size = "hello"
    .size()
```

## Visibility

Supported today:

- `hidden` on top-level `def`
- `hidden` on top-level immutable bindings
- `hidden` on top-level `interface`
- `hidden` on top-level `class` / `shape` / `object` / `enum`
- `hidden` on fields
- `hidden` on methods

Default visibility is public. There is no `public` keyword.
Use `hidden` for private top-level functions, constants, types, fields, and methods.
Top-level mutable bindings are not allowed; mutable module state must live inside
named objects, class instances, or function locals.

## Notes

This file is meant to describe the current surface syntax.

Ideas that are still under discussion belong in `features.md`, not here.
