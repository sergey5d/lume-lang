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
- `[K : V]` (map; nominal spelling `Map[K, V]` is also accepted)
- `Set[T]`
- `Vector[T]` or `[T]`
- `LinkedList[T]`

Vector and map shorthand can be nested, for example:

- `[[Int]]`
- `[[[(Str, Int)]]]`
- `[Str : [Int]]`
- `[Str : [Int : Bool]]`

Common stdlib/prelude types:

- `Option[T]`
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

Universal value operations:

- `value.toStr()` returns a `Str` rendering of the value
- `value.equals(other)` returns `Bool` and has the same equality semantics as `value == other`

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

Function types:

- `fn(Int) => Str`
- `fn(Int, Bool) => Unit`
- `fn() => Unit`

Function types use `fn` directly before a parenthesized parameter-type list.
Write `fn(Int) => Int`, not `(Int) => Int`, `Int => Int`, or `fn (Int) => Int`.
The return type may itself be a function type, as in
`fn() => fn(Int) => Str`. Lambda expressions remain keyword-free, for example
`value => value + 1`.

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

Chains normally stay lifted and are consumed by `try`, `let ... else`,
`if let`, or `match`:

```txt
name = try userOpt.->profile.->name
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
let Some(nameField) = classType.field("name") else panic("expected name field")
println(nameField.fieldType().name() !!)
println(nameField.isHidden())

enumType EnumType[Status] = typeOf[Status].asEnum() !!
let Some(pendingCase) = enumType.case("Pending") else panic("expected Pending case")
println(pendingCase.name())
constructedCase Result[Any, ReflectionError] = pendingCase.construct()
```

Safe reflective invocation uses `Result` values:

```txt
constructed Result[User, ReflectionError] = classType.construct("Ada", 42)

user User = constructed !!
nameValue Result[Any, ReflectionError] = nameField.get(user)

let Some(greetMethod) = classType.method("greet") else panic("expected greet method")
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

Annotations are not supported on reserved `impl` placeholders.

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
- `impl TypePattern`
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

Arbitrary statements such as `if`, `for`, `match`, `defer`, or expression statements are not valid at top level. Put executable code inside a function such as `main() { ... }`.

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
    location: "New York"
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
- anonymous shapes use `{ field: value }` for field construction and `shape(...)` for contextual positional construction
- builtin constructor forms such as `Vector(...)`, `Array(...)`, and `Range(...)` use parentheses
- `Type { ... }` resolves through the available explicit `new(...)` declaration or implicit field-construction inputs
- `Type(...)` resolves through explicit class `new(...)`, implicit visible-field construction, named shape positional construction, or builtin constructor forms
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
{ expr }                         # block expression
Type { field: value }            # brace field construction or enum field payload
call { x => ... }                # trailing lambda
Interface with Other { method(...) ... } # anonymous interface implementation
object { field Type = value; method() Type = value } # anonymous object
object with Interface { method() Type = value } # explicit anonymous interface implementation
new(field Type)                  # constructor declaration
shape(value, other)              # contextual anonymous-shape positional construction
```

Single-expression braces such as `{ value }` are block expressions, not anonymous shapes. To construct an anonymous shape, use construction fields with `:`.

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

`def` is optional for top-level functions, local functions, and methods. Both
forms are valid, and declarations are recognized by the callable header shape
`name[TypeParams](params)`:

```txt
greet(name Str) Str = "hello, " + name
def greet(name Str) Str = "hello, " + name
```

The parameter list is attached to the callable name. `name(...)` starts a
callable declaration; `name (...)` does not. Function-valued bindings carry
the explicit `fn` type marker, as in `mapper fn(Int) => Int`.

Expression-bodied function:

```txt
greet(name Str) Str = "hello, " + name
```

Block-bodied function:

```txt
add(left Int, right Int) Int {
    return left + right
}

addWithEquals(left Int, right Int) Int = {
    return left + right
}
```

Callable block bodies may include `=` or omit it. Expression-bodied callables
still use `=`.

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
    typeOf[A].name() !!

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
callable-parameter    = name Type [vararg]
                      | name => Type
constructor-parameter = name Type [vararg] [= default]
```

Rules:

- `vararg` is postfix-only; the removed prefix form `vararg values [T]` is invalid.
- A variadic parameter must have an explicit vector type `[T]`.
- Only the final parameter may be variadic.
- A parameter list may contain at most one variadic parameter.
- A by-name parameter cannot also be variadic.
- A variadic constructor parameter may have a default vector value, written after `vararg`.

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
- Use an explicit `fn() => T` parameter when the caller should pass, store, or return the thunk itself.

Style:

- Use by-name parameters only for conditional-value APIs such as `assert`, `debug`, `getOr`, and `orElse`.
- Use `fn() => T` for callbacks, schedulers, retry operations, event handlers, and stored work.

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
    case SomeX(x) => x + 1
    case NoneX => 0
})
```

The same idea applies to `partial match`:

```txt
options.map(value => partial match value {
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
- callable block bodies may be written as `name(...) { ... }` or `name(...) = { ... }`
- if you want a block value, the last statement must be value-producing
- value-producing tail forms include ordinary expressions, `if / else`, `match`, and `for ... yield`
- blocks can nest arbitrarily

## Classes, Shapes, Objects, Interfaces, Enums

Class:

```txt
class Box[T] with Named {
    value T
    label() Str = "box"
}
```

When a class, shape, enum, or named object implements an interface method inside
its body, it uses an ordinary method declaration. `def` is optional.

`impl` is reserved for future generic specialization. It currently has no
specialization behavior and its body must be empty:

```txt
impl Either[T, T] {}
impl Option[Str] {}
```

Constructors and methods are never declared in `impl`; put them directly in the
owning type declaration body.

Named object:

```txt
object MathBox {
    value Int = 5

    valuePlusOne() Int = this.value + 1
    double(value Int) Int = value * 2
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

    describe() Str = this.label + ": " + this.count.toStr()
}
```

Rules:

- every anonymous-object field has an initializer
- anonymous-object fields are immutable; use a named class for owned mutable state
- fields appear before methods
- anonymous objects cannot declare `new` constructors
- fields and methods are statically typed and use ordinary member access
- `this.field` and unqualified `field` are both available inside methods

To explicitly implement an interface, use either the interface-led form or the
equivalent object-led form:

```txt
greeter Greeter = Greeter {
    greet() Str = "hello"
}

other Greeter = object with Greeter {
    greet() Str = "hello"
}
```

Another class example:

```txt
class Amount with Named {
    value Int
    label Str
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

`LinkedList {}` is the empty constructor; non-empty values use positional
`LinkedList(...)` construction. The old indexed `get` and `remove` methods are
not part of the collection API.

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

emptyNames() [Str] = []
emptyCounts() [Str : Int] = []

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
if let Some(item) = maybeValue {
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

`if let` is intended for refutable matches. If the compiler can prove the
pattern always succeeds for the scrutinee type, it rejects the construct and
asks you to use plain `let` instead.

When the payload needs more destructuring, prefer doing that on the next line inside the branch:

```txt
if let Some(pair) = maybePair {
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
let Some(item) = maybeValue else {
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
- `let Some(item) = maybeValue else { ... }` for `Option[T]`
- `let Ok(item) = maybeResult else { ... }` for `Result[T, E]`
- `let Right(item) = maybeEither else { ... }` for `Either[L, R]`

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
    Some(left) = maybeLeft
    Some(right) = maybeRight
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
let Some(item) = maybeValue else panic("expected Some")
let item <- maybeValue else panic("expected value")
```

Grouped assertive extraction uses the same `let { ... } else` form:

```txt
let {
    Some(left) = maybeLeft
    Some(right) = maybeRight
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
enclosing loop and cannot jump across lambda or lifted-access callback
boundaries. `continue` and `break` inside `for ... yield` are valid only for
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

`!!` ends its postfix chain. Further calls, indexing, or member access require
an explicit group:

```txt
name = wrapped !!.name       # invalid
name = (wrapped !!).name     # valid
```

Multiple dependent unwraps can be written as sequential `let ... else` / `try`
statements or as a grouped `let` block with `else`:

```txt
left = try maybeLeft

let Some(right) = maybeRight else {
    return Err("missing")
}

let {
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
if let Some(left) = maybeLeft && let Ok(right) = compute() && right > left {
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
    let Some(item) = maybeItem else {
        continue
    }
    println(item)
}
```

These generator heads are invalid:

```txt
for (x, y) <- pairs { ... }
for { name, age } <- users { ... }
for Some(item) <- values { ... }
for let Some(item) <- values { ... }
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
    case Some(value) => value
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
    case SomeX(x) => {
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
values.map(value => partial match value {
    case SomeX(x) => x + 1
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
    case Log(message) => {
        println(message)
    }
    case Other(message) => println(message)
}
```

If no case matches, `partial match` returns `None`.

Supported pattern families:

- wildcard: `_`
- binding pattern: `x`
- literal/value patterns: `1`, `"hello"`, `true`
- case alternatives: `case A | B => ...`
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

Rules:

- enum exhaustiveness is checked
- expression `partial match` skips exhaustiveness checking and wraps the result in `Option[...]`
- statement `partial match` skips exhaustiveness checking and does nothing when no case matches
- bare zero-payload enum cases should still be written in qualified form when needed, for example `MaybeInt.NoneX`

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
- `<-` for `for` iteration and success-case extraction in `if let` and `let ... else`
- `??` for extract-or-fallback through `Option`, `Result`, and `Either`
- `!!` for unsafe extraction through `Option`, `Result`, and `Either`
- `fn(...) => T` for function types
- `=>` for lambdas and by-name parameters
- `=>` for match cases
- `.->` for per-hop lifted access through `Option`, `Result`, and `Either`
- `with` for interface implementation and generic bounds
- `:` inside map types, map literals, and construction field lists
- `:<` for class, shape, and anonymous-shape update

Examples:

```txt
counter is Counter
for item <- items {
}
fn(Int) => Str
SomeX(x) => x
class Box[T] with Named
pair = ("a", 1)
name = maybeName ?? "unknown"
```

Shape copy, extension, and distinct merge:

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
  - equality operators: `==`, `!=`
  - symbolic collection/custom forms: `:+`, `:-`, `++`, `--`, `::`
- Comparison operators are intended to work through `Ordering[T]` rather than custom operator declarations.
- Equality is intended to work through `Eq[T]` rather than custom operator declarations.
- Standard collections do not define symbolic operators like `:+`, `:-`, `++`, or `--`; collection APIs should prefer searchable method names.
- `:` is brace-entry syntax only, not an overloadable operator.
- The spellings `:+`, `:-`, `++`, `--`, and `::` are removed from the language surface and currently produce `unsupported_operator` lexer diagnostics.

Newline continuation:

- Ordinary expressions are no longer broadly newline-insensitive.
- A newline continues the current expression only when the previous line clearly ends in a continuation form, except postfix chains may continue when the next line starts with `.` or `.->`.
- Continuation tokens:
  - binary operators: `+`, `-`, `*`, `/`, `%`, `&&`, `||`, `==`, `!=`, `<`, `<=`, `>`, `>=`
  - extraction/fallback operators: `??`
  - shape/update operators: `:<`
  - unary prefixes: unary `-`, `!`, `try`
  - runtime type check keyword: `is`
  - match arrow: `=>`
  - separators / chaining markers: `,`, `.`, `.->`
- Delimited forms allow layout after opening delimiters and after commas, but they do not make leading binary/update operators valid by themselves.
- Binding/callable `=` may start its expression on the same line or the next indented line.
- Callable bodies have three accepted forms:
  - `name(...) { ... }` for block bodies
  - `name(...) = { ... }` for block bodies
  - `name(...) = expr` for expression bodies
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
