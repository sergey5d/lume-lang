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

Built-in generic/container types:

- `Array[T]`
- `Map[K, V]`
- `Set[T]`
- `List[T]` or `[T]`

List shorthand can be nested, for example:

- `[[Int]]`
- `[[[(Str, Int)]]]`

Common stdlib/prelude types:
- `Any`
- `Option[T]`
- `Result[T, E]`
- `Either[L, R]`
- `Iterable[T]`
- `Iterator[T]`
- `Type[T]`
- `Type[_]`
- `TypeKind`
- `Ordering[T]`
- `Printer`
- `OS`

## Wildcard Capture

`_` inside a type argument is an existential capture, not a normal concrete type.
It means "some definite type, but this code does not know which one."

```txt
a List[_] = List(1, 2, 3)
b Map[_, Str] = intStrMap()
c Map[_, _] = strIntMap()
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
same(a[0], b[0])      # rejected if b is a different List[_] source
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

Universal value operations:

- `value.toStr()` returns a `Str` rendering of the value
- `value.equals(other)` returns `Bool` and has the same equality semantics as `value == other`
- `Any` is the top value type: any value can be assigned to `Any`, but `Any` is not assignable back to a narrower type without an explicit safe form

Tuple types:

- `(Int, Str)`

Function types:

- `(Int) -> Str`
- `(Int, Bool) -> Unit`

Function type parameter lists must be parenthesized. Use `(Int) -> Int`,
not `Int -> Int`. Lambda expressions still use ordinary arrow syntax, for
example `value -> value + 1`.

## Lifted Access Operator

Use `.->` to access a member of the value inside a lifted container:
`Option[T]`, `Result[T, E]`, or `Either[L, T]`. The lifted access operator is
one token; whitespace inside `.->` is invalid.

Each `.->` is one hop. The member resolves against the inner success type,
together with any immediately following call or index postfixes:

```txt
firstName = userOpt
    .->profile
    .->name()
    .->first
```

Rules:

- `x.->m` requires `x` to be `Option`, `Result`, or `Either`; `m` resolves
  against the inner type
- a hop is one member plus its immediate `(...)` / `[...]` postfixes; the
  next `.` or `.->` starts the next hop
- if the hop result is plain, the hop lowers to `map`
- if the hop result is the same lifted family the hop lowers to
  `flatMap`
- otherwise the result nests; there is no cross-family flattening
- plain `.m` on a lifted value is an ordinary method call on the container
  type itself, for example `userOpt.->name().getOr("unknown")`

```txt
userOpt.->profileOpt().->name   # inner access on every hop
userOpt.->profileOpt().name     # error: Option[Profile] has no member
                                # 'name'; use '.->name'
```

Chains normally stay lifted and are consumed by `try`, `guard`,
`if let`, or `match`:

```txt
name = try userOpt.->profile.->name
```

## Lift Expression

Use `lift` to turn a shape or tuple whose members are all the same success
container family into one container of the assembled value.

```txt
profile Option[{ id Int, name Str }] = lift {
    id: maybeId()
    name: maybeName()
}

pair Result[(Int, Str), Str] = lift (okId(), okName())

spreadProfile Option[{ id Int, name Str }] = lift {
    ...optionParts
}
```

Rules:

- `lift { ... }` accepts named shape fields only
- shape spread is allowed; spread fields are checked as lifted members in source order
- `lift (...)` accepts tuple literals
- every member must be `Option[T]`, `Result[T, E]`, or `Either[L, T]`
- all members must use the same wrapper family
- `Result` error types and `Either` left types must be mutually compatible
- empty shapes and tuples are rejected because the wrapper family cannot be inferred

The result keeps the same wrapper family:

```txt
lift { id: Option[Int], name: Option[Str] }
# Option[{ id Int, name Str }]

lift (Result[Int, E], Result[Str, E])
# Result[(Int, Str), E]

lift { id: Either[L, Int], name: Either[L, Str] }
# Either[L, { id Int, name Str }]
```

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
Type[_]   # metadata for some captured unknown represented type
Type[Any] # exact typed metadata specifically for Any
```

`typeOf[T]` returns `Type[T]`. `value.runtimeType` returns `Type[A]` for a
concrete statically known value type `A`. If the value is statically `Any` or
otherwise not known precisely, `runtimeType` returns `Type[_]`.

Common metadata operations:

```txt
println(typeOf[User].name().orPanic())
println(typeOf[User].kind())

classType ClassType[User] = typeOf[User].asClass().orPanic()
fields = classType.fields()
expect Some(nameField) = classType.field("name")
println(nameField.fieldType().name().orPanic())
println(nameField.isHidden())

enumType EnumType[Status] = typeOf[Status].asEnum().orPanic()
expect Some(pendingCase) = enumType.case("Pending")
println(pendingCase.name())
constructedCase Result[Any, ReflectionError] = pendingCase.construct()
```

Safe reflective invocation uses `Result` values:

```txt
constructed Result[User, ReflectionError] = classType.construct("Ada", 42)

user User = constructed.orPanic()
nameValue Result[Any, ReflectionError] = nameField.get(user)

expect Some(greetMethod) = classType.method("greet")
greeting Result[Any, ReflectionError] = greetMethod.call(user)
```

`Method.invoke(receiver, args...)` is also available as the direct invocation
form and returns `Any`; it may panic if the method is not invokable or the call
fails. Prefer `call` when failures should stay in the value model.

Rules:

- `typeOf[T]` is a built-in type metadata operator, not an index operation
- `runtimeType` is available as a read-only synthetic field on values
- `TypeKind` includes `Class`, `Shape`, `Enum`, `Interface`, `Single`, `Annotation`, `Primitive`, `Tuple`, `Function`, and `AnonymousShape`
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

Extension methods are imported only by wildcard module use. They are visible in
the module where the `ext` block is declared and in files that write
`use module/sub/*`. Selective imports such as `use module/sub/Name` do not import
extension methods.

The standard spec helper module is imported explicitly by test files. It is not
part of the prelude; specs are executed by the test runner:

```txt
use spec/*

class PrimitiveSpec with Spec {}

impl PrimitiveSpec {
    def it() Unit {
        5.shouldBe(5)
        "ok".shouldBe("ok")
    }
}
```

`spec` provides `Spec` and primitive `shouldBe` extension methods. A failed
`shouldBe` panics. `lume test file.lum` discovers every class or single that
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

single Routes {
    health Str = "/health"
}

@Route { path: routePath }
def status() Str = "ok"

@Route { path: "/health" }
def health() Str = "ok"

@Route { path: Routes.health }
def healthFromSingle() Str = "ok"

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

`single Routes { ... }` declares one singleton value named `Routes`, so `Routes.health`
is ordinary field access on that stable singleton value.

Annotation arguments are compile-time metadata values. They may only be literals, stable constants, aggregate literals made from allowed values, or constant expressions composed from allowed values:

- immutable top-level constants, including imported constants
- immutable fields on `single` values, such as `Routes.health`; this is allowed because `single Name { ... }` declares the singleton value `Name`
- immutable constants through a module alias, such as `routes.healthPath`
- enum cases, such as `RouteVisibility.External`
- arithmetic, comparison, boolean, and string-concatenation expressions whose operands are also annotation-safe

Calls, constructors, indexing, mutable singleton fields, ordinary object field reads, `try`, `for ... yield`, `match`, `if`, lambdas, and blocks are rejected in annotation arguments. Top-level mutable bindings are not allowed at all, so they are rejected before annotation argument checking.

Supported targets currently include:

- top-level `def`, `annotation`, `interface`, `class`, `shape`, `single`, `enum`
- fields
- methods
- interface methods
- enum cases

Annotations on `impl` blocks themselves are not supported; annotate the methods inside the block instead.

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
- `single`
- `enum`
- `ext TypeName`
- `name Type = expr`
- `hidden def`
- `hidden name Type = expr`
- `hidden annotation`
- `hidden interface`
- `hidden class`
- `hidden shape`
- `hidden single`
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

single Counter {
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

Arbitrary statements such as `if`, `for`, `match`, `defer`, `guard`, `expect`, or expression statements are not valid at top level. Put executable code inside a function such as `main() { ... }`.

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
inside a `single`, class instance, or function local.

Fields without initializers are only valid in class-like field declarations:

```txt
class Box {
    hidden var cached Int
    hidden label Str
}
```

`hidden` fields in classes and singles may infer their type from an initializer:

```txt
class Box {
    hidden count = 0
    hidden var hits = 0
}

single Greeter {
    hidden hello = "Hello"
}
```

Visible class fields, shape fields, enum fields, and singleton fields still require explicit field types.

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

Shape update:

```txt
updated = value :< {
    age: 42
    name: "Bob"
}

patch = { age: 43 }
updated2 = value :< patch
```

Anonymous-shape spread:

```txt
copy = { ...value }

extended = {
    ...value
    location: "Tampa"
}

merged = {
    ...namePart
    ...agePart
}
```

Spread entries copy fields from a class, shape, or anonymous-shape value into a
new anonymous shape. Spread is additive only: duplicate field names are an
error, including an explicit field after a spread. To update an existing field,
use `:<`.

`base :< patch` updates existing visible fields. `base` must be a class, named
shape, or anonymous shape. `patch` must be a statically known shape-like value.
Every visible field in `patch` must already exist on `base`, and each patch
field type must be assignable to the corresponding base field type. The result
keeps the same class/shape view as `base`. Hidden fields are not updated through
`:<`.

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

Anonymous shapes use plain braces for construction fields:

```txt
user = {
    name: "Ada"
    age: 10
}
```

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

Tuple-to-shape construction is contextual. The target shape must be known from
an annotation, parameter type, or return type:

```txt
user { name Str, age Int } = ("Ada", 10)

def makeUser() { name Str, age Int } {
    return ("Ada", 10)
}

describe(("Cara", 14))
```

Tuples do not construct classes. Classes must name their constructor target:

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
- methods are declared with `impl ShapeName { ... }`
- custom `new` constructors are not allowed
- shapes may declare interface bounds with `shape Name with Interface`
- brace field construction uses `ShapeName { field: value }`
- positional construction uses `ShapeName(...)`
- contextual tuple construction is allowed when the expected type is a known shape

```txt
shape Point {
    x Int
    y Int
}

impl Point {
    def sum() Int = this.x + this.y
}

interface Named {
    def label() Str
}

shape NamedPoint with Named {
    x Int
    y Int
}

impl NamedPoint {
    def label() Str = this.x + "," + this.y
}

origin = Point(0, 0)
named = Point { x: 3, y: 4 }
tupled Point = (5, 6)
```

Construction rules:

General construction rules:

- a constructor shape declares the fields accepted by construction
- constructor parentheses accept positional arguments only; use braces for construction fields
- function and method calls may still use named arguments in parentheses
- `Type { value }` is not valid; use `Type(value)`
- anonymous shapes use `{ field: value }` for field construction and tuple values for contextual positional construction
- builtin constructor forms such as `List(...)`, `Array(...)`, and `Range(...)` use parentheses
- `Type { ... }` and `Type(...)` both resolve through the available explicit `new` shape or implicit field-construction shape
- class construction is nominal and constructor-gated; shape construction is structural
- tuple values cannot construct classes; write `User(...)` or `User { ... }`
- tuple values can construct anonymous or named shapes only when the target shape type is known
- nested inner constructions must still name the target class explicitly, often by binding the inner value first, for example `leader = Person { name: "Ada", age: 10 }` and then `owner = Team { leader: leader }`
- `Type({ ... })` is not supported; use construction fields or positional values directly
- `shape(...)` expression syntax is not supported; use tuple-to-shape construction instead

Explicit constructor shape rules:

- `new { field Type, other Type = default } { ... }` declares an explicit constructor input shape with required and defaulted fields
- constructor input fields do not have to be class fields; they are inputs to the constructor body
- `Type { field: value, other: value }` matches the explicit constructor input shape by field name
- `Type(value, otherValue)` fills the same explicit constructor input shape by declaration order
- constructor input fields may be declared in the author's preferred order; defaults do not have to trail required fields
- named construction may omit any constructor input field that has a default
- positional construction fills a prefix of constructor input fields in declaration order
- positional construction may omit only a trailing suffix whose fields all have defaults
- positional construction never skips a defaulted field to reach a later required field
- if any explicit `new` exists, implicit field construction is disabled for that class
- explicit constructors may use one trailing variadic constructor shape field such as `vararg items [T]`
- a variadic constructor shape field receives the extra positional arguments as `[T]`
- only one variadic constructor shape field is allowed
- construction fields can target a variadic constructor shape field by passing a `[T]` value
- variadic constructor shape fields may have a default `[T]` value

```txt
class Article {
    body Str
    title Str
}

impl Article {
    new {
        body Str = "body"
        title Str
    } {
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

- if a class has no explicit `new`, the compiler synthesizes a constructor shape from visible fields
- construction braces check the synthesized visible-field shape
- visible fields without initializers are required
- visible fields with initializers are optional
- hidden fields are excluded from the synthesized constructor shape
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
{ expr }                         # block expression
Type { field: value }            # brace field construction or enum field payload
call { x -> ... }                # trailing lambda
Interface with Other { method(...) ... } # anonymous interface implementation
new { field Type }               # constructor input shape declaration
```

Single-expression braces such as `{ value }` are block expressions, not anonymous shapes. To construct an anonymous shape, use construction fields with `:`.

Shape conversion rules:
- field names and field types must match at compile time
- extra fields are allowed when passing a value to a narrower shape
- missing fields are rejected
- defaults are not part of the shape syntax
- tuple-to-shape is allowed only when the target shape is known
- tuple-to-shape follows shape field order exactly
- shape-to-shape assignment is structural by field names and field types
- class-to-shape is allowed through visible fields
- shape-to-interface follows the shape's explicit `with Interface` bounds
- class-to-interface-through-shape is not automatic; assign the class value to an explicit shape view first
- hidden class fields are not visible to shape conversion
- shape-to-class is not implicit; use a class constructor
- tuple-to-class is not allowed; use a class constructor
- ordinary calls may still accept named anonymous shapes in parentheses, for example `describe({ name: "Cara", age: 14 })`
- construction fields inside braces use `field: value`
- construction fields may carry an explicit initializer type as `field Type: value`
- single-expression braces like `{ value }` are still block expressions, not anonymous shapes

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

point Point = (1, 2)                # tuple -> named shape
anon { x Int, y Int } = (1, 2)      # tuple -> anonymous shape
fromClass Point = Pixel(1, 2)       # class -> shape
named Point = { x: 1, y: 2 }        # anonymous shape -> named shape

user User = ("Ada", 10)             # invalid: tuple -> class
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

`def` is optional for top-level functions and methods. Both forms are valid,
and declarations are recognized by the callable header shape
`name[TypeParams](params)`:

```txt
greet(name Str) Str = "hello, " + name
def greet(name Str) Str = "hello, " + name
```

The parameter list is attached to the callable name. `name(...)` starts a
callable declaration; `name (...)` does not, which keeps fields such as
`pair (Int, Int)` and `mapper (Int) -> Int` unambiguous.

Expression-bodied function:

```txt
greet(name Str) Str = "hello, " + name
```

Block-bodied function:

```txt
add(left Int, right Int) Int {
    return left + right
}
```

Callable block bodies omit `=`. A block expression is valid in ordinary
expression positions, but not directly after a callable-body `=`.

Generic function:

```txt
id[T](value T) T = value
```

Generic bounds:

```txt
sort[T with Ordering[T]](value T) T = value
```

Reified generic functions and methods:

```txt
typeName[reified A](value A) Str =
    typeOf[A].name().orPanic()

metadata[reified A]() Type[A] =
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

Function and method parameters may end with one variadic list parameter. `vararg`
is written before the parameter name:

```txt
println(vararg value [Str]) Unit
printf(format Str, vararg value [Str]) Unit
```

The parameter is available as `[T]` inside the body, and call sites pass the
extra values positionally.

Function and method parameters may be marked `lazy`:

```txt
def twice(lazy value Int) Int =
    value + value

def debug(lazy message Str) Unit
```

Rules:

- `lazy` is allowed on function and method parameters only.
- A lazy argument expression is captured as a zero-argument closure.
- Reading the parameter evaluates that closure.
- Lazy parameters are not memoized; each read evaluates the captured expression again.
- Lazy parameters cannot be `vararg`.
- Lazy argument expressions cannot contain non-local `return`, `break`, `continue`, or `try`.
- Use an explicit `() -> T` parameter when the caller should pass, store, or return the thunk itself.

Style:

- Use `lazy` only for conditional-value APIs such as `assert`, `debug`, `getOr`, and `orElse`.
- Use `() -> T` for callbacks, schedulers, retry operations, event handlers, and stored work.

Forwarding rules:

```txt
def inner(lazy value Int) Int = value

def outer(lazy value Int) Int =
    inner(value)
```

- Passing a lazy parameter to a normal parameter evaluates it first.
- Passing a lazy parameter to another lazy parameter forwards the delayed expression.
- Reading a lazy parameter in any other expression evaluates it immediately.

If a callee needs one evaluation, bind the value explicitly:

```txt
def cached(lazy value Int) Int {
    item = value
    item + item
}
```

Core fallback APIs use `lazy` parameters so fallback work only runs on the
fallback branch. Mapper callbacks such as `map`, `flatMap`, `mapLeft`, and
`mapError` are ordinary function values; only the callback body is conditional
on the container branch:

```txt
value = maybe.getOr(expensiveDefault())
result = maybe.toResult(makeError())
next = result.orElse(recover())
mapped = maybe.map(value -> value + 1)
leftMapped = either.mapLeft(error -> error.toStr())
```

Classes, enums, and singles can declare methods inline. Classes, shapes, enums, and singles can also attach behavior through top-level `impl` blocks. Shape bodies remain data-only, so shape methods must use `impl ShapeName { ... }`:

```txt
class Counter {
    value Int
}

impl Counter {
    inc() Int = this.value + 1
}
```

Extension methods attach receiver-call syntax from outside the target type's
own implementation:

```txt
ext Counter {
    doubled() Int = this.value * 2
}

counter = Counter { value: 3 }
println(counter.doubled())
```

Extension rules:

- extension blocks use `ext TypeName { method(...) ... }`; `def` is still accepted but optional
- extension targets may be classes, shapes, enums, interfaces, or built-in primitive types such as `Int`, `Float`, `Bool`, `Str`, and `Rune`
- extension targets cannot be singles, annotations, or enum cases
- extension blocks cannot declare constructors
- a module may declare multiple `ext` blocks for the same target type
- extension methods use the same call syntax as regular methods
- `this` is the extended receiver
- extension methods can access only the visible members available from the extension module
- extension methods are visible in their declaring module and in files that import that module with `use module/*`
- extension imports are file-local; importing a module that imports extensions does not re-export those extension methods

```txt
use model/user/{User}
use model/user_extensions/*

user User = User { name: "Ada" }
label = user.displayName()
```

Custom constructors are class-only and use a dedicated `new` block inside `impl`.

- `new { ... }` declares the constructor input shape
- `new { ... } { body }` declares a block-bodied constructor
- `new { ... } = expression` declares an expression-bodied constructor
- shape, enum, enum case, single, annotation, and interface declarations cannot define custom `new` constructors
- constructor shape fields use `name Type`, with optional defaults such as `age Int = 0`
- `Type { field: value }` constructs by matching the constructor input shape by field name
- `Type(value)` constructs by filling the same constructor input shape positionally by declaration order
- named construction may omit any constructor shape field with a default
- positional construction fills a prefix of constructor shape fields and may omit only trailing fields that all have defaults
- positional construction never skips a defaulted field to reach a later required field
- constructor shape fields may end with one variadic list field such as `vararg items [Str]`
- `hidden new { ... } { body }` declares a private constructor
- each explicit class constructor must initialize every field that does not have a field initializer, or delegate to another constructor
- `new(...)` inside another constructor delegates positionally to another constructor of the same class
- `new { field: value }` inside another constructor delegates with construction fields to another constructor of the same class
- class call sites use braces for construction fields, for example `Person { name: "Ada", age: 10 }`
- class call sites use parentheses for positional arguments, for example `Person("Ada", 10)`
- `this` is the instance receiver
- instance fields on classes, enums, and singles may be accessed bare when they are not shadowed
- use `this.field` when a parameter/local shadows a field, for example `this.age`
- member order is storage first, constructors next, methods last
- class, shape, enum, and single bodies list storage fields before behavior
- enum cases count as enum storage and must appear before enum methods
- class impl blocks list all `new` constructors before ordinary methods

```txt
class Person {
    age Int
    name Str
}

impl Person {
    new {
        age Int
        name Str
    } {
        this.age = age
        this.name = name
    }

    new {
        age Int
    } = new(age, "unknown")

    new {
        name Str
    } = new {
        age: 0
        name: name
    }
}
```

Variadic constructor shape fields collect positional arguments into a `[T]`
inside the constructor body:

```txt
class Path {
    segments [Str]
}

impl Path {
    new {
        vararg segments [Str] = ["tmp"]
    } {
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
() -> expr
x -> expr
_ -> expr
(x) -> expr
(x, y) -> expr
(x Int) -> expr
(x Int, y Int) -> expr
(_) -> expr
(x, _) -> expr
(_ Int, value Int) -> expr
```

Typed single-parameter lambdas must use parentheses, so write
`(x Int) -> x + 1`, not `x Int -> x + 1`. Parenthesized parameter lists
must also be either fully typed or fully untyped; `(x Int, y) -> ...` is
invalid. Plain `(x, y) -> ...` always means two parameters.

Single-parameter lambda:

```txt
x -> x + 1
```

Explicitly typed lambda:

```txt
(x Int) -> x + 1
```

Multi-parameter lambda:

```txt
(left Int, right Int) -> left + right
```

Tuple-destructuring inside a one-argument lambda:

```txt
pairs.map(pair -> {
    let (key, value) = pair
    key + value
})

pairs.map(pair -> {
    let (key, _) = pair
    key
})
```

Class or anonymous-shape destructuring inside a lambda:

```txt
users.map { user ->
    let { name, age } = user
    "$name is $age"
}
```

Lambda parameters cannot use `let` destructuring. If a lambda receives a tuple,
class, or anonymous-shape value, name the parameter normally and destructure it
inside the body:

```txt
pairs.mapWithIndex((pair, index) -> {
    let (x, y) = pair
    "$index: ${x + y}"
})

source.combine((name, pair) -> {
    let (x, y) = pair
    "$name: ${x + y}"
})

source.combine((left, right) -> {
    let (a, b) = left
    let (x, y) = right
    a + b + x + y
})
```

Rules:

- `_` inside an explicit lambda parameter list means "ignore this parameter slot"
- `_` is not a readable value, so `(_, value) -> _ + value` is invalid
- `_ -> expr` is valid as a one-parameter lambda whose parameter is ignored
- placeholder-expression lambdas such as `_ + 1` and `items.map(_ + 1)` are not supported
- tuple, class, and anonymous-shape values are destructured inside the lambda body with normal `let`
- `let` destructuring is not allowed in lambda parameter lists

Callable references can be passed where a function value is expected. They are
eta-expanded to the same forwarding lambda you would otherwise write:

```txt
def mapUser(user User) UserDto =
    UserDto { id: user.id, name: user.name }

dtos = users.map(mapUser)
# same as: users.map(user -> mapUser(user))

mapper UserMapper = UserMapper()
dtos = users.map(mapper.mapUser)
# same as: users.map(user -> mapper.mapUser(user))

dtos = users.map(this.mapUser)
# same as: users.map(user -> this.mapUser(user))

dtos = users.map(User.toDto)
# single method reference
```

Supported callable references:

- top-level function name
- bound instance method, such as `mapper.mapUser`
- bound `this` method, such as `this.mapUser`
- bound `single` method, such as `User.toDto`

Fields still win over methods when names collide. A member field whose value is
already a function is passed as that function value, not eta-expanded as a
method reference.

Block lambda:

```txt
(x Int) -> {
    value := x + 1
    value
}
```

Trailing block-lambda call syntax is also allowed when passing a lambda as an argument. For lambdas with parameters, the lambda head must start on the same line as the opening `{`:

```txt
items.map { x -> x + 1 }

items.repeat { () -> 5 }

items.zipMap { (left, right) -> left + right }

items.forEach { x ->
    next = x + 1
    println(next)
}

items.map { (x Int) ->
    x + 1
}

items.zipMap { (left,
    right) -> left + right }
```

If the expected argument type is a zero-argument function, the lambda head may
be omitted and the trailing block is treated as `() -> { ... }`:

```txt
process {
    println("hehe")
}
```

If a callback is passed alongside ordinary arguments, include it in the same
parenthesized argument list. Do not write a trailing block after an already
completed `(...)` call; that would imply currying or calling the result of the
first call.

```txt
processNamed("compares values", {
    println("inside callback")
})
```

Trailing brace call syntax on non-constructor calls is only for lambda arguments.
Constructor braces fill the constructor shape by field name, so enum named
payloads use braces and enum positional payloads use parentheses:

```txt
maybeOrder = Some(Order { id: 7 })
namedMaybeOrder = Some { value: Order { id: 7 } }
```

If the body after `->` starts on the next line, it may be either:
- a single expression spread over later lines
- or a multi-statement lambda body without an extra `{ ... }` wrapper

Use an explicit lambda when mapping with a `match`:

```txt
options.map(value -> match value {
    case SomeX(x) => x + 1
    case NoneX => 0
})
```

The same idea applies to `partial match`:

```txt
options.map(value -> partial match value {
    case SomeX(x) => x + 1
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
- block expressions are not valid directly after callable-body `=`; write `name(...) { ... }` for a callable block body
- if you want a block value, the last statement must be value-producing
- value-producing tail forms currently include ordinary expressions, `if / else`, `match`, and `for ... yield`
- blocks can nest arbitrarily

## Classes, Shapes, Singles, Interfaces, Enums

Class:

```txt
class Box[T] with Named {
    value T
}

impl Box[T] {
    label() Str = "box"
}
```

When a class or singleton implements an interface method inside its body or an `impl ... { ... }` block, it uses an ordinary method declaration. `def` is optional.

Singleton:

```txt
single MathBox {
    value Int = 5

    valuePlusOne() Int = this.value + 1
}

impl single MathBox {
    double(value Int) Int = value * 2
}

box = MathBox
answer = box.valuePlusOne()
```

`single Name { ... }` declares both a singleton type `Name` and one singleton value `Name`.
The expression `Name` evaluates to that singleton value, so singles can be passed to functions, stored in locals, and called through later like any other value.
Singles cannot be constructed with `Name()` or `Name {}`; reference `Name` directly.

`impl single Name { ... }` attaches methods only to an explicit `single Name { ... }` declaration. It never creates the singleton by itself; declare `single Name {}` first when no fields are needed.

Another class example:

```txt
class Amount with Named {
    value Int
    label Str
}

impl Amount {
    label() Str = this.label
}
```

Interfaces:

```txt
interface Named {
    label() Str
}
```

Interfaces may also provide default methods by attaching a body:

```txt
interface Named {
    label() Str
    greeting() Str = "Hello " + this.label()
}
```

Methods that satisfy an interface just use ordinary method declarations:

```txt
interface Named {
    label() Str
}

class Box with Named {
}

impl Box {
    label() Str = "box"
}
```

Anonymous interface implementation expressions:

```txt
handler = Reader with Closer {
    read() Str = "x"
    close() Unit = ()
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
- there is no `impl Enum.Case { ... }` form
- zero-payload cases are values and are written without call syntax, for example `None`
- payload cases use positional constructor syntax, for example `Some(value)`
- payload cases may also use construction fields in braces, for example `Some { value: value }`
- payload and shared fields with defaults may be omitted from enum case constructors
- `None()`-style calls for zero-payload cases are invalid

Behavior for enums belongs on the enum itself, either inline or in `impl Enum { ... }` blocks, and case-specific behavior should be expressed with `match`.

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

## Lists, Arrays, Maps, Tuples

List literal:

```txt
[1, 2, 3]
["a", "b"]
```

Array construction:

```txt
ints Array[Int] = Array.ofInt(3)       # [0, 0, 0]
floats Array[Float] = Array.ofFloat(3) # [0.0, 0.0, 0.0]
bools Array[Bool] = Array.ofBool(3)    # [false, false, false]
texts Array[Str] = Array.ofStr(3)      # ["", "", ""]
runes Array[Rune] = Array.ofRune(3)    # default NUL rune values

filled Array[Int] = Array.fill(3, 7)
generated Array[Int] = Array.generate(3, idx -> idx * 2)
```

Arrays always contain initialized values. Use `Array.generate` when each slot
should be produced independently.

Array elements can also be constructed directly:

```txt
values Array[Int] = Array(1, 2, 3)
boxes Array[Box] = Array(Box(1), Box(2))
takeArray(Array(4, 5, 6))
```

Map construction:

```txt
entries Map[Str, Int] = Map("a": 1, "b": 2)
value Option[Int] = entries["a"]
```

`Map(...)` accepts tuple-pair arguments. The `key: value` form is a general
pair expression, not syntax that only exists inside `Map(...)`.

Tuple literal:

```txt
(1, "x")
pair (Str, Int) = "a": 1
```

`:` has two roles depending on grammar context:

- in brace field lists, `field: value` binds a value to a construction field
- in ordinary expressions, `left: right` constructs a 2-tuple pair value

Pair expressions are non-associative. Use `(a, b, c)` for TupleN values, and
use parentheses when intentionally nesting pairs: `(a: b): c` or `a: (b: c)`.

Pair values inside field initializers should be parenthesized for readability:

```txt
holder = Holder {
    entry: ("a": 1)
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
- `guard`
- `expect`
- `return`
- `break`
- `continue`
- expression statement

Pure expression statements with no effect are rejected.

Standalone nested blocks are valid expression statements:

```txt
{
    OS.println("xxx")
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
    OS.println("closing")
}
```

Only a call expression or a block is allowed after `defer`. Deferred blocks may
not contain `return`, `break`, or `continue`.

## `if`

Statement form:

```txt
if value > 0 {
    OS.println("positive")
} else {
    OS.println("non-positive")
}
```

Pattern-test form:

```txt
if let Some(item) = maybeValue {
    OS.println(item)
}
```

`if let` also accepts the shorthand for the success case:

```txt
if let item <- maybeValue {
    OS.println(item)
}
```

Runtime type patterns also work in `if let`:

```txt
if let worker Worker = value {
    OS.println(worker)
}

if let _ Worker = value {
    OS.println("value is a Worker")
}
```

`if let` is intended for refutable matches. If the compiler can prove the
pattern always succeeds for the scrutinee type, it rejects the construct and
asks you to use plain `let` instead.

When the payload needs more destructuring, prefer doing that on the next line inside the branch:

```txt
if let Some(pair) = maybePair {
    let (x, y) = pair
    OS.println(x)
    OS.println(y)
}
```

Direct nested payload destructuring in the `if let` pattern itself is still a possible future extension.

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

If the pattern can fail, plain `let` is rejected. Use a refutable binding form
instead.

`guard ... else` is the refutable binding form with an explicit fallback path:

```txt
guard Some(item) = maybeValue else {
    return Err("missing")
}
```

For success-carrying values, `<-` is shorthand for the success case:

```txt
guard item <- maybeValue else {
    return Err("missing")
}
```

This is equivalent to:
- `guard Some(item) = maybeValue else { ... }` for `Option[T]`
- `guard Ok(item) = maybeResult else { ... }` for `Result[T, E]`
- `guard Right(item) = maybeEither else { ... }` for `Either[L, R]`

The shorthand requires the source type to be statically known as one of these
forms. If the source type is unknown, use an explicit pattern instead.

Type-pattern binding is also supported:

```txt
guard worker Worker = value else {
    return Err("wrong kind")
}

guard _ Worker = value else {
    return Err("wrong kind")
}
```

Grouped refutable bindings share one fallback:

```txt
guard {
    Some(left) = maybeLeft
    Some(right) = maybeRight
} else {
    return Err("missing")
}
```

`guard ... else` is statement-oriented:
- the pattern is matched against the right-hand value
- if the match succeeds, bindings remain visible after the statement
- if the match fails, the `else` block is evaluated and must exit the current control-flow path, typically with `return`, `break`, `continue`, or a call whose return type is `Never`

Success-case extraction shorthand such as `let item <- maybeValue` is accepted
without `else` only when the source expression itself proves the successful
case:

```txt
let item <- Some(5)        # ok: source is visibly Some

maybe Option[Int] = Some(5)
let item <- maybe          # error: maybe has type Option[Int], so extraction can fail
```

`expect` is the assertive refutable binding form. It matches the pattern, binds
on success, and panics on mismatch:

```txt
expect Some(item) = maybeValue
```

If the binding is irrefutable, use `let` instead. That includes patterns that
are irrefutable from the value type and patterns whose source expression visibly
proves success:

```txt
expect item <- Some(5)     # error: use let item <- Some(5)
```

And the matching shorthand:

```txt
expect item <- maybeValue
```

Grouped `expect` works the same way:

```txt
expect {
    Some(left) = maybeLeft
    Some(right) = maybeRight
}
```

`expect` is statement-only and does not support `else`; use `guard ... else`
when you want an explicit fallback path.

Use the runtime/prelude `assert(...)` function for plain boolean assertions:

```txt
assert(split.size() == 3)
assert(split.size() == 3, "split must have 3 parts")
```

The first argument must be `Bool`. When the condition is `false`, `assert`
panics. The optional second argument is the panic message.
Statement-style `assert condition` is not supported.
Boolean checks are intentionally not written with `expect`; `expect` is reserved
for pattern/assertive binding.

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
row = try Db.query(id).mapError { err -> AppError.Db(err) }
value = try sourceEither.mapLeft { left -> AppError.FromLeft(left) }
```

`try` propagates the value it receives. If the source has the wrong failure type,
transform the container first:

- `Option[T].toResult(error)` converts absence into `Err(error)`.
- `Result[T, E].mapError(f)` maps `Err(E)` into another error type.
- `Either[L, R].mapLeft(f)` maps `Left(L)` into another left type.

When the chain gets visually noisy, split before the mapping call:

```txt
row = try Db.query(id)
    .mapError { err -> AppError.Db(err) }
```

Multiple dependent unwraps can be written as sequential `guard ... else` / `try`
statements or as a grouped `guard` block:

```txt
left = try maybeLeft

guard Some(right) = maybeRight else {
    return Err("missing")
}

guard {
    Some(left) = maybeLeft
    Some(right) = maybeRight
} else {
    return Err("missing")
}
```

`if let` also supports a grouped form:

```txt
if let {
    Some(left) = maybeLeft
    Some(right) = maybeRight
} {
    OS.println(left + right)
}
```

And grouped clauses can use `<-` too:

```txt
if let {
    left <- maybeLeft
    right <- maybeRight
} {
    OS.println(left + right)
}
```

And `if let` conditions can be chained with `&&` so later clauses can use
earlier bindings:

```txt
if let Some(left) = maybeLeft && let Ok(right) = compute() && right > left {
    OS.println(left + right)
}
```

Only `&&` joins are supported in this form.

## `for`

Simple loop:

```txt
for item <- [1, 2, 3] {
    OS.println(item)
}
```

Range loop:

```txt
for i <- Range(0, 10) {
    OS.println(i)
}
```

`Range(start, end)` is start-inclusive and end-exclusive. With two arguments it automatically chooses a step of `1` or `-1` based on the bounds, and `Range(start, end, step)` allows an explicit step.

Generator heads normally bind one plain identifier:

```txt
for row <- rows {
    OS.println(row)
}
```

Use `for let` for explicitly marked irrefutable tuple or shape
destructuring:

```txt
for let (x, y, char) <- rows {
    OS.println(char)
}
```

The same rule applies to class and anonymous-shape values. Shape
destructuring matches by field name, not by position:

```txt
for let { name, location } <- users {
    OS.println(name, location)
}

for let { location as loc, name } <- users {
    OS.println(name, loc)
}
```

Refutable logic goes in the loop body:

```txt
for maybeItem <- items {
    guard Some(item) = maybeItem else {
        continue
    }
    OS.println(item)
}
```

These generator heads are invalid:

```txt
for (x, y) <- pairs { ... }
for { name, age } <- users { ... }
for Some(item) <- values { ... }
for let Some(item) <- values { ... }
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

Only these clause kinds are allowed inside `for { ... } yield`:

```txt
name <- iterable
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

Refutable `guard`, `expect`, reassignment, mutation, expression
statements, and guards are not clause forms. Put that logic in the body or use
helpers such as `filterMap`:

```txt
result = items.filterMap(item -> partial match item {
    case Some(value) => value
})
```

Mental model:

```txt
for      = pulls values from iterables
let      = destructures irrefutable values
guard    = exits early from refutable bindings in the loop body
match    = handles refutable cases
yield    = produces values
```

`for item <- items yield item * 2` lowers approximately to
`items.map(item -> item * 2)`.

Nested generators lower approximately through `flatMap` and `map`:

```txt
for {
    x <- xs
    y <- ys
} yield x + y
```

is approximately:

```txt
xs.flatMap(x ->
    ys.map(y ->
        x + y
    )
)
```

`continue` is valid in `while`, `for`, and `for ... yield`.
Inside `for ... yield`, it skips the current iteration without producing a
value.

Condition-controlled loops use `while`:

```txt
while current < 10 {
    current += 1
}
```

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
    OS.println(item)
}
```

## `match`

Statement form:

```txt
match value {
    case SomeX(x) => {
        OS.println(x)
    }
    case OptionX.NoneX => {
        OS.println("none")
    }
}
```

Expression form:

```txt
result = match value {
    case SomeX(x) => x
    case OptionX.NoneX => 0
}
```

Guards are supported on cases with `if ... =>`:

```txt
result = match value {
    case SomeX(x) if x > 10 => x
    case SomeX(_) => 10
    case OptionX.NoneX => 0
}
```

Partial match expression form:

```txt
result Option[Int] = partial match value {
    case SomeX(x) => x
}
```

Partial match statement form executes the matching case if one exists and does
nothing when no case matches:

```txt
partial match value {
    case SomeX(x) => println(x)
}
```

Partial mapped through an explicit lambda:

```txt
values.map(value -> partial match value {
    case SomeX(x) => x + 1
})
```

`match` and `partial match` always require an explicit value and a block of cases.
Omitted-scrutinee shorthand such as `match { ... }` is not supported.
Postfix match syntax such as `value match { ... }` is not supported; use prefix
`match value { ... }`.
Inline `match value: ...` shorthand is not supported.

Every `match` and `partial match` branch must start with `case`.

Every case must have an explicit body after `=>`: an expression, `()` for Unit, or a block such as `{}`.

```txt
match value {
    case Skip => ()
    case Empty => {}
    case Log(message) => {
        OS.println(message)
    }
    case Other(message) => OS.println(message)
}
```

If no case matches, `partial match` returns `None`.

Supported pattern families:

- wildcard: `_`
- binding pattern: `x`
- literal/value patterns: `1`, `"hello"`, `true`
- tuple patterns: `(x, y)`
- enum constructor patterns: `SomeX(x)`
- class extractor patterns: `PairBox(left, right)`
- type patterns: `item Worker`, `_ Other`

Type patterns use erased outer-type matching at runtime. For generic declared types, match on the outer name only:

```txt
match value {
    _ Box => ...
    _ Bag => ...
}
```

Generic arguments inside runtime type patterns are intentionally rejected for now, so use `_ Box` rather than `_ Box[Int]`.

Current notes:

- enum exhaustiveness is checked
- expression `partial match` skips exhaustiveness checking and wraps the result in `Option[...]`
- statement `partial match` skips exhaustiveness checking and does nothing when no case matches
- bare singleton enum cases should still be written in qualified form when needed, for example `MaybeInt.NoneX`

## Destructuring

Tuple destructuring:

```txt
let (left Int, right Str) = (5, "hello")
```

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
- `lift` for assembling shapes or tuples inside `Option`, `Result`, or `Either`
- `<-` for `for` iteration and success-case extraction in `if let`, `guard ... else`, and `expect`
- `->` for parenthesized function types and lambdas
- `=>` for match cases
- `.->` for per-hop lifted access through `Option`, `Result`, and `Either`
- `with` for interface implementation and generic bounds
- `:` for ordinary pair expressions, where `left: right` constructs a 2-tuple
- `:<` for class, shape, and anonymous-shape update

Examples:

```txt
counter is Counter
for item <- items {
}
(Int) -> Str
SomeX(x) => x
class Box[T] with Named
pair = "a": 1
```

Shape copy, extension, and distinct merge:

```txt
copy = { ...user }
extended = {
    ...user
    location: "Tampa"
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
  - equality operators: `==`, `!=`
  - symbolic collection/custom forms: `:+`, `:-`, `++`, `--`, `::`
- Comparison operators are intended to work through `Ordering[T]` rather than custom operator declarations.
- Equality is intended to work through `Eq[T]` rather than custom operator declarations.
- Standard collections do not define symbolic operators like `:+`, `:-`, `++`, or `--`; collection APIs should prefer searchable method names.
- `:` is a built-in pair expression operator only, not an overloadable collection/custom operator.
- The spellings `:+`, `:-`, `++`, `--`, and `::` are removed from the language surface and currently produce `unsupported_operator` lexer diagnostics.

Newline continuation:

- Ordinary expressions are no longer broadly newline-insensitive.
- A newline continues the current expression only when the previous line clearly ends in a continuation form, except postfix chains may continue when the next line starts with `.` or `.->`.
- Continuation tokens:
  - binary operators: `+`, `-`, `*`, `/`, `%`, `&&`, `||`, `==`, `!=`, `<`, `<=`, `>`, `>=`
  - shape/update operators: `:<`
  - unary prefixes: unary `-`, `!`, `try`, `lift`
  - runtime type check keyword: `is`
  - match arrow: `=>`
  - separators / chaining markers: `,`, `.`, `.->`
- Delimited forms allow layout after opening delimiters and after commas, but they do not make leading binary/update operators valid by themselves.
- Binding/callable `=` may start its expression on the same line or the next indented line.
- Callable bodies have two forms:
  - `name(...) { ... }` for block bodies
  - `name(...) = expr` for expression bodies
  - `name(...) = { ... }` is invalid even though blocks are expressions elsewhere; callable block bodies omit `=`
  - `def name(...) ...` remains accepted as the explicit keyword form
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

updated = user :<
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

name = userOpt
    .->profileOpt()
    .->name()
    .->first
```

## Visibility

Supported today:

- `hidden` on top-level `def`
- `hidden` on top-level immutable bindings
- `hidden` on top-level `interface`
- `hidden` on top-level `class` / `shape` / `single` / `enum`
- `hidden` on fields
- `hidden` on methods

Default visibility is public. There is no `public` keyword.
Use `hidden` for private top-level functions, constants, types, fields, and methods.
Top-level mutable bindings are not allowed; mutable module state must live inside
`single`, class instances, or function locals.

## Notes

This file is meant to describe the current surface syntax.

Ideas that are still under discussion belong in `features.md`, not here.
