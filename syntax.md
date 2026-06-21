# Syntax Reference

This file describes the language syntax that is available now.

## Built-In Data Types

Primitive types:

- `Int`
- `Int64`
- `Float`
- `Float64`
- `Bool`
- `Str`
- `Rune`
- `Unit`

Built-in generic/container types:

- `Array[T]`
- `Map[K, V]`
- `Set[T]`
- `List[T]` or `[T]`
- `Unit`

List shorthand can be nested, for example:

- `[[Int]]`
- `[[[(Str, Int)]]]`

Common stdlib/prelude types:
- `Option[T]`
- `Iterable[T]`
- `Iterator[T]`
- `Ordering[T]`
- `Printer`
- `OS`

Tuple types:

- unnamed tuples: `(Int, Str)`

Function types:

- `(Int) -> Str`
- `(Int, Bool) -> Unit`

Function type parameter lists must be parenthesized. Use `(Int) -> Int`,
not `Int -> Int`. Lambda expressions still use ordinary arrow syntax, for
example `value -> value + 1`.

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
OS.panic("boom")
OS.stdout.println("hello")
OS.stderr.println("oops")
```

`OS.stdout` and `OS.stderr` implement `Printer`.

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
  use all public symbols unqualified
- `use module/sub/A`
  use one symbol unqualified
- `use module/sub/A as B`
  use one symbol with a local alias
- `use module/sub/{A, B as D, C}`
  use a selected symbol set
- `use module/sub/Object/*`
  use all visible singleton methods unqualified
- `use module/sub/Object/{printLn as printN, print}`
  use selected visible singleton methods from a singleton

Built-in `OS` methods are available implicitly in every file, so `print(...)`, `println(...)`, `printf(...)`, and `panic(...)` work without writing `use OS/*`. Fields like `OS.stdout` and `OS.stderr` still use explicit member access.

## Top-Level Declarations

Annotations use `@` followed by a normal constructor call, typically for a class type. They are parsed and attached to declarations and members as metadata.

Examples:

```txt
class Route {
    path Str
}

@Route(path = "/health")
def health() Str = "ok"

@Route("/health")
def health2() Str = "ok"
```

Supported targets currently include:

- top-level `def`, `class`, `single`, `enum`, `interface`
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
- `interface`
- `class`
- `single`
- `enum`
- `public def`
- `public name Type = expr`
- `hidden def`
- `hidden interface`
- `hidden class`
- `hidden single`
- `hidden enum`

Examples:

```txt
def greet(name Str) Str = "hello, " + name

interface Named {
    def label() Str
}

class Box[T] {
    value T
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

Top-level bindings are also supported:

```txt
seed Int = 1
var counter Int = 0
```

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

Public fields, records, and enums still require explicit field types.

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

Inside `new`, direct writes to `this.field` use `=` even for `var` fields or
fields that already have defaults. `:=` and compound assignment are for
post-construction mutation.

Member reassignment:

```txt
this.count := this.count + 1
```

Index assignment:

```txt
values[0] := 1
values[1] := values[0] + 4
```

Record update:

```txt
updated = value with {
    age: 42
    name: "Bob"
}
```

Anonymous record literal:

```txt
user = { name: "Ada", age: 10 }
```

Positional anonymous record construction is also allowed when the target shape is already known:

```txt
user { name Str, age Int } = { "Ada", 10 }
```

Multiline anonymous record literal:

```txt
user = {
    name: "Ada"
    age: 10
}
```

Inferred field type from a local value:

```txt
a = 1
b = {
    count: a
}
```

Mixed separators are also valid:

```txt
user = { name: "Ada",
    age: 10
}
```

Anonymous record shape type:

```txt
def describe(user { name Str, age Int }) Str =
    user.name + " is " + user.age
```

Positional construction also works for shaped parameters and shaped return values:

```txt
def makeUser() { name Str, age Int } {
    return { "Ada", 10 }
}

describe({ "Cara", 14 })
```

Named classes use the same brace construction surface:

```txt
user User = User { name: "Ada", age: 10 }
person Person = Person { "Ben", 12, "NYC" }
profile MixedProfile = MixedProfile {
    name: "Liam"
    age: 8
}
tail HiddenTail = HiddenTail { "Ada", 4 }
settings Settings = Settings {}
```

Rules for field-based class construction:
- `Type(...)` is never synthesized from fields
- `Type(...)` only works when the target class defines `new(...)` or the target is a builtin constructor form such as `List(...)`
- `Type { ... }` is the structural construction form
- any explicit `new(...)` disables structural brace construction for that class
- `Type {}` works when the visible construction shape has no required fields
- named braces check only visible public fields
- in named braces, public fields without initializers are required
- in named braces, public fields with initializers are optional
- in named braces, hidden fields are never part of the accepted shape
- hidden fields without initializers block structural construction entirely
- positional braces follow declared public-field order
- positional braces may omit only a trailing suffix of public fields that already have initializers
- positional braces are rejected when a hidden initialized field appears before a later public field
- mutable vs immutable field differences do not matter for structural shape matching
- named class values do not structurally convert to other named class values
- nested inner constructions must still name the target class explicitly, often by binding the inner value first, for example `leader = Person { name: "Ada", age: 10 }` and then `owner = Team { leader: leader }`
- `Type({ ... })` is not supported; class structural construction must use `Type { ... }`

Anonymous record shapes are structural:
- field names and field types must match at compile time
- extra fields are allowed when passing a value to a narrower shape
- missing fields are rejected
- defaults are not part of the shape syntax
- construction uses plain `{ ... }` in expression position
- ordinary calls may still accept anonymous records in parentheses, for example `describe({ "Cara", 14 })`
- named fields inside construction braces use `field: value`
- named fields may carry an explicit initializer type as `field Type: value`
- `{ value1, value2 }` is positional and requires an anonymous record shape from context
- single-expression braces like `{ value }` are still block expressions, not anonymous records
- inside `{ ... }`, fields may be separated by commas, newlines, or a mix of both

Typed anonymous record fields:

```txt
user = {
    name Str: "Ada"
    age Int: 42
}
```

## Functions and Methods

Expression-bodied function:

```txt
def greet(name Str) Str = "hello, " + name
```

Block-bodied function:

```txt
def add(left Int, right Int) Int {
    return left + right
}
```

Generic function:

```txt
def id[T](value T) T = value
```

Generic bounds:

```txt
def sort[T with Ordering[T]](value T) T = value
```

Objects and enums can declare methods inline. Classes and records attach behavior through top-level `impl` blocks:

```txt
class Counter {
    value Int
}

impl Counter {
    def inc() Int = this.value + 1
}
```

Constructors currently use `def new(...)`.

- `new(...)` declares a constructor
- `new(...)` inside another constructor delegates to another constructor of the same class
- `this` is the instance receiver
- instance fields must be accessed through `this.`, for example `this.age`

```txt
class Person {
    age Int
    name Str
}

impl Person {
    def new(age Int, name Str) {
        this.age = age
        this.name = name
    }

    def new(age Int) = new(age, "unknown")
}
```

## Lambdas

Accepted lambda parameter forms are deliberately small:

```txt
() -> expr
x -> expr
(x) -> expr
(x, y) -> expr
(x Int) -> expr
(x Int, y Int) -> expr
```

Typed single-parameter lambdas must use parentheses, so write
`(x Int) -> x + 1`, not `x Int -> x + 1`. Parenthesized parameter lists
must also be either fully typed or fully untyped; `(x Int, y) -> ...` is
invalid.

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

Tuple-destructuring lambda in a one-argument function context:

```txt
pairs.map((key, value) -> key + value)
pairs.map((key, _) -> key)
```

Rules:

- if a lambda is expected to take one argument and that argument is a tuple, `(a, b) -> ...` destructures that tuple into separate names
- the same syntax still means a normal multi-parameter lambda when the contextual function type expects multiple arguments
- `_` inside an explicit lambda parameter list means "ignore this parameter slot"

Block lambda:

```txt
(x Int) -> {
    value := x + 1
    value
}
```

Trailing block-lambda call syntax is also allowed when passing a lambda as an argument:

```txt
items.map { x -> x + 1 }

items.forEach {
    x ->
        next = x + 1
        println(next)
}
```

Trailing brace call syntax is only for lambda arguments. Ordinary arguments,
including enum constructor payloads, must still use parentheses:

```txt
maybeOrder = Some(Order { id: 7 })
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

The same idea applies to `partial`:

```txt
options.map(value -> partial value {
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
- if you want a block value, the last statement must be value-producing
- value-producing tail forms currently include ordinary expressions, `if / else`, `match`, and `for ... yield`
- blocks can nest arbitrarily

## Classes, Objects, Interfaces, Enums

Class:

```txt
class Box[T] with Named {
    value T
}

impl Box[T] {
    def label() Str = "box"
}
```

When a class or singleton implements an interface method inside an `impl ... { ... }` block, it uses ordinary `def`.

Singleton:

```txt
single MathBox {
}

impl single MathBox {
    def double(value Int) Int = value * 2
}
```

`impl single Name { ... }` may also synthesize an empty singleton companion when `Name` already exists and no singleton fields are needed.

Another class example:

```txt
class Amount with Named {
    value Int
    label Str
}

impl Amount {
    def label() Str = this.label
}
```

Interfaces:

```txt
interface Named {
    def label() Str
}
```

Interfaces may also provide default methods by attaching a body:

```txt
interface Named {
    def label() Str
    def greeting() Str = "Hello " + this.label()
}
```

Methods that satisfy an interface just use ordinary `def`:

```txt
interface Named {
    def label() Str
}

class Box with Named {
}

impl Box {
    def label() Str = "box"
}
```

Anonymous interface implementation expressions:

```txt
handler = Reader with Closer {
    def read() Str = "x"
    def close() Unit = ()
}
```

Enums:

```txt
enum Color {
    code Str

    def isWarm() Bool = code == "red"

    case Red {
        code = "red"
    }
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
- there is no `impl Enum.Case { ... }` form
- zero-payload cases are values and are written without call syntax, for example `None`
- payload cases use call syntax, for example `Some(value)`
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

## Lists, Arrays, Tuples

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

Tuple literal:

```txt
(1, "x")
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

Expression form:

```txt
result = if value > 0 {
    1
} else {
    0
}
```

Brace-delimited branches are the preferred `if` form. `else` does not require `:`.

## `let ... else` and `try`

Preferred refutable binding form:

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

Plain pattern `let` is only allowed for irrefutable matches:

```txt
pair (Int, Int) = (1, 2)
let (left, right) = pair
```

If the match can fail, plain `let` is rejected and you must use `let ... else`.
That includes success-case extraction shorthand such as `let item <- maybeValue`,
which is always treated as refutable.

An explicit assertive form is also supported:

```txt
expect Some(item) = maybeValue
```

And the matching shorthand:

```txt
expect item <- maybeValue
```

Grouped assertive `expect` works the same way:

```txt
expect {
    Some(left) = maybeLeft
    Some(right) = maybeRight
}
```

`expect` matches the pattern, binds on success, and panics on mismatch.
`expect` is statement-only and does not support `else`; use `let ... else`
when you want an explicit fallback path.

`expect` also supports plain boolean assertions:

```txt
expect split.size() == 3
```

This form requires a `Bool` condition and panics when the condition is `false`.

Possible future extension:

```txt
expect split.size() == 3, "asset must have 3 parts"
expect Some(value) = maybeValue, "missing value"
```

This is only a proposal for now; trailing messages on `expect` are not currently implemented.

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
- `Result[T, E]` may propagate from `Result[..., E2]` when `E` is assignable to `E2`
- `Either[L, R]` may propagate from `Either[L2, ...]` when `L` is assignable to `L2`

The success type may differ; the propagated failure side must still be compatible.

Multiple dependent unwraps can be written as sequential `let ... else` / `try`
statements or as a grouped `let` block:

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

Destructuring loop:

```txt
for (x, y, char) <- rows {
    OS.println(char)
}
```

Class destructuring loop:

```txt
for { name, location } <- users {
    OS.println(name, location)
}

for { location as loc, name } <- users {
    OS.println(name, loc)
}
```

Pattern loop:

```txt
allSome = [Some(5), Some(6)]
for Some(item) <- allSome {
    OS.println(item)
}
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
    y <- [10, 20]
} yield {
    x + y
}
```

`yield` also accepts a same-line expression without `:`:

```txt
items = for item <- [1, 2, 3] yield item * 2
```

`for` clauses in the block form may also include local `=` bindings and `:=`
updates.

Tuple destructuring in `for` clauses must use parentheses:

```txt
for (value, idx) <- rows {
    OS.println(value, idx)
}
```

Class destructuring in `for` clauses uses braces and follows the same
name-based rules as `let { ... }`.

Pattern-based `for` clauses are supported when the compiler can prove that
every produced value matches the pattern. That proof may come from the item type
being irrefutable for the pattern, or from an exact known iterable such as a
literal list of matching values. If the compiler can see a non-matching
alternative, it rejects the loop.

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

Partial expression form:

```txt
result Option[Int] = partial value {
    case SomeX(x) => x
}
```

Partial mapped through an explicit lambda:

```txt
values.map(value -> partial value {
    case SomeX(x) => x + 1
})
```

`match` and `partial` always require a block of cases. Inline `match value: ...` shorthand is not supported.

Every `match` and `partial` branch must start with `case`.

If no case matches, `partial` returns `None`.

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
- `partial` skips exhaustiveness checking and wraps the result in `Option[...]`
- bare singleton enum cases should still be written in qualified form when needed, for example `MaybeInt.NoneX`

## Destructuring

Tuple destructuring:

```txt
let (left Int, right Str) = (5, "hello")
```

Anonymous record / class-shape destructuring uses braces:

```txt
let { value Int, label Str } = { value: 7, label: "world" }
```

Class destructuring also uses braces:

```txt
let { left Int, right Str } = Box { 9, "boxed" }
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

Brace destructuring for classes and anonymous records matches by field name.
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

The same name-based rule applies in `for { ... } <- items`.

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
- `<-` for `for` iteration and success-case extraction in `if let`, `let ... else`, and `expect`
- `->` for parenthesized function types and lambdas
- `=>` for match cases
- `with` for interface implementation, generic bounds, and record update

Examples:

```txt
counter is Counter
for item <- items {
}
(Int) -> Str
SomeX(x) => x
class Box[T] with Named
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

Newline continuation:

- Ordinary expressions are no longer broadly newline-insensitive.
- A newline continues the current expression only when the previous line clearly ends in a continuation form.
- Continuation tokens:
  - binary operators: `+`, `-`, `*`, `/`, `%`, `&&`, `||`, `==`, `!=`, `<`, `<=`, `>`, `>=`
  - bitwise operators: `|`, `&`
  - match arrow: `=>`
  - separators / chaining markers: `,`, `.`
- Continuation is also allowed inside unmatched delimiters:
  - `(...)`
  - `{...}`
  - `[...]`
- Assignment-style operators require a right-hand side on the same line:
  - `=`
  - `:=`
  - `+=`, `-=`, `*=`, `/=`, `%=`
  - `<-`
- Callable bodies have two forms:
  - `def name(...) { ... }` for block bodies
  - `def name(...) = expr` for expression bodies
  - `def name(...) = { ... }` is invalid when `{ ... }` is a statement block; omit `=`
- `=` may start its expression on the same line or the next indented line:
  - `a =` followed by a newline is valid
  - `def value() Int =` followed by a newline is valid for expression bodies
  - inline-body introducers such as `else` and `yield` may take a same-line body without braces
  - if that body moves to the next line, a `{ ... }` block is required
- So this is valid:

```txt
a =
    1 + 2
```

- while this stays valid:

```txt
a = 1 +
    2
```

- and this also stays valid:

```txt
def value() Int =
    1 + 2

if flag {
    return 1
}
```

- For dot chaining, the rule is stricter than Scala:
  - allow newline after `.`
  - do not rely on newline before `.`

## Visibility

Supported today:

- `public` on top-level `def`
- `public` on top-level immutable bindings
- `hidden` on top-level `def`
- `hidden` on top-level `interface`
- `hidden` on top-level `class` / `single` / `enum`
- `hidden` on fields
- `hidden` on methods

Top-level `def` and immutable bindings are private by default and only become importable across modules when marked `public`.

## Notes

This file is meant to describe the current surface syntax.

Ideas that are still under discussion belong in `features.md`, not here.
