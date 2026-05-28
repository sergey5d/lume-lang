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
- `List[T]`
- `Unit`

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

## Strings

Ordinary string literals use double quotes:

```txt
"hello"
```

String interpolation is supported in any string:

```txt
"hello $name"
"next ${count + 1}"
"money \$5"
```

Rules:

- `$name` interpolates a simple identifier expression
- `${...}` interpolates a full expression
- `\$` inserts a literal dollar sign
- `Str.size()` returns the string length as `Int`

Multiline strings use triple quotes:

```txt
"""
hello
world
"""
```

Rules:

- multiline strings preserve their contents exactly, including a leading newline
- multiline strings do not interpolate
- `"""` strings must not be empty

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

## Imports

Supported import forms:

```txt
import module/sub
import module/sub/*
import module/sub/A
import module/sub/A as B
import module/sub/{A, B as D, C}
```

Meaning:

- `import module/sub`
  qualified access through the module name, for example `sub.A`
- `import module/sub/*`
  import all public symbols unqualified
- `import module/sub/A`
  import one symbol unqualified
- `import module/sub/A as B`
  import one symbol with a local alias
- `import module/sub/{A, B as D, C}`
  import a selected symbol set
- `import module/sub/Object/*`
  import all visible object methods unqualified
- `import module/sub/Object/{printLn as printN, print}`
  import selected visible object methods from an object

Built-in `OS` methods are imported implicitly in every file, so `print(...)`, `println(...)`, `printf(...)`, and `panic(...)` work without writing `import OS/*`. Fields like `OS.stdout` and `OS.stderr` still use explicit member access.

## Top-Level Declarations

Annotations use `@` followed by a normal constructor call, typically for a record type. They are parsed and attached to declarations and members as metadata.

Examples:

```txt
record Route {
    path Str
}

@Route(path = "/health")
def health() Str = "ok"

@Route("/health")
def health2() Str = "ok"
```

Supported targets currently include:

- top-level `def`, `class`, `object`, `record`, `enum`, `interface`
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
- `object`
- `record`
- `enum`
- `public def`
- `public name Type = expr`
- `hidden def`
- `hidden interface`
- `hidden class`
- `hidden object`
- `hidden record`
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

object Counter {
    var count Int = 0
}

record Amount {
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

Deferred / uninitialized fields are only valid in class-like field declarations:

```txt
class Box {
    hidden var cached Int
    hidden label Str = ?
}
```

`hidden` fields in classes and objects may infer their type from an initializer:

```txt
class Box {
    hidden count = 0
    hidden var hits = 0
}

object Greeter {
    hidden hello = "Hello"
}
```

Public fields, records, and enums still require explicit field types.

## Assignment and Update

Reassignment:

```txt
count := count + 1
```

Compound assignment:

```txt
count += 1
count -= 1
count *= 2
count /= 2
count %= 2
```

Member assignment:

```txt
this.value = value
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
    age = 42
    name = "Bob"
}
```

Anonymous record literal:

```txt
user = record { name = "Ada", age = 10 }
```

Positional anonymous record construction is also allowed when the target shape is already known:

```txt
user { name Str, age Int } = record("Ada", 10)
```

Multiline anonymous record literal:

```txt
user = record {
    name = "Ada"
    age = 10
}
```

Inferred field type from a local value:

```txt
a = 1
b = record {
    count = a
}
```

Mixed separators are also valid:

```txt
user = record { name = "Ada",
    age = 10
}
```

Anonymous record shape type:

```txt
def describe(user { name Str, age Int }) Str =
    user.name + " is " + user.age
```

Positional construction also works for shaped parameters and shaped return values:

```txt
def makeUser() { name Str, age Int } = {
    return record("Ada", 10)
}

describe(record("Cara", 14))
```

Named records/classes can be built from an anonymous record only through an explicit type call:

```txt
userRecord = record {
    name = "Ada"
    age = 10
}

user User = User(userRecord)
person Person = Person(record("Ben", 12, "NYC"))
profile MixedProfile = MixedProfile {
    name = "Liam"
    age = 8
}
```

Rules for this conversion:
- it applies only when the single argument is an anonymous record value
- the anonymous record shape must match the visible construction shape of the target class/record
- field names and field types must match exactly
- public fields without initializers are required
- public fields with initializers are optional and may be provided or omitted
- private fields with initializers are not part of the accepted shape and may not be provided
- private fields without initializers block this conversion entirely
- mutable vs immutable field differences do not matter for shape matching
- explicit constructors are ignored for this conversion path
- named record/class values do not structurally convert to other named record/class values
- nested inner conversions must still be explicit, for example `owner = Person(record { ... })`
- `Type { ... }` is allowed as sugar for passing a single anonymous record argument
- `Type({ ... })` is not supported; plain `{ ... }` inside call parentheses is treated as a block, not as an anonymous record literal

Anonymous record shapes are structural:
- field names and field types must match at compile time
- extra fields are allowed when passing a value to a narrower shape
- missing fields are rejected
- defaults are not part of the shape syntax
- construction uses `record { ... }`; plain `{ ... }` remains a block expression
- `record(...)` is allowed only when an anonymous record shape is known from context
- inside `record { ... }`, fields may be separated by commas, newlines, or a mix of both

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
    def inc() Int = value + 1
}
```

Constructors currently use `def init(...)`.

- `init(...)` declares a constructor
- `init(...)` inside another constructor delegates to another constructor of the same class
- `this` is only the instance receiver, for example `this.age`

```txt
class Person {
    age Int
    name Str
}

impl Person {
    def init(age Int, name Str) {
        this.age = age
        this.name = name
    }

    def init(age Int) = init(age, "unknown")
}
```

## Lambdas

Single-parameter lambda:

```txt
x -> x + 1
```

Underscore shorthand in a lambda-expected context:

```txt
inc (Int) -> Int = _ + 1
items.map(_ + 1)
```

Rules:

- if an expression containing `_` appears where a one-argument function is expected, it expands to a lambda
- `_ + 1` becomes `x -> x + 1`
- the shorthand is contextual; outside a lambda-expected position, `_` is not a normal value
- this also works with larger expressions such as `items.map(if _ > 5 then 10 else 8)` or `items.map(match _ { ... })`

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
- the placeholder shorthand rule for `_ + 1` only applies when there is no explicit `->` lambda parameter list

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
```

Contextual `match` lambda sugar is also allowed in a unary-function context:

```txt
options.map(match {
    case SomeX(x) => x + 1
    case NoneX => 0
})
```

`match { ... }` in that position desugars to `match _ { ... }`. The same also works for `partial { ... }`.

```txt
options.map(partial {
    case SomeX(x) => x + 1
})
```

`partial { ... }` in that position desugars to `partial _ { ... }`.

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

## Classes, Objects, Records, Interfaces, Enums

Class:

```txt
class Box[T] with Named {
    value T
}

impl Box[T] {
    def label() Str = "box"
}
```

When a class, record, or object implements an interface method inside an `impl Type { ... }` block, it uses ordinary `def`.

Object:

```txt
object MathBox {
    def double(value Int) Int = value * 2
}
```

Records:

```txt
record Amount with Named {
    value Int
    label Str
}

impl Amount {
    def label() Str = label
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
- `impl Enum.Case { ... }` is not supported

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
values Array[Int] = Array.ofLength(3)
values[0] := 1
```

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
- `return`
- `break`
- expression statement

Pure expression statements with no effect are rejected.

Standalone nested blocks are valid expression statements:

```txt
{
    OS.println("xxx")
}
```

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
- if the match fails, the `else` block is evaluated and implicitly returns from the current callable

Propagation form:

```txt
item = try maybeValue
```

`try` unwraps the success side of:
- `Option[T]`
- `Result[T, E]`
- `Either[L, R]`

and returns early with the original failure value when the source is empty / error / left.

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
for x, y, char <- rows {
    OS.println(char)
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

`for` clauses in the block form may also include local `=` and `:=` bindings.

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

Contextual partial form:

```txt
values.map(partial {
    case SomeX(x) => x + 1
})
```

`match` and `partial` always require a block of cases. Inline `match value: ...` shorthand is not supported.

Every `match` and `partial` branch must start with `case`, including contextual forms like `match { ... }` and `partial { ... }`.

If no case matches, `partial` returns `None`.

Supported pattern families:

- wildcard: `_`
- binding pattern: `x`
- literal/value patterns: `1`, `"hello"`, `true`
- tuple patterns: `(x, y)`
- enum constructor patterns: `SomeX(x)`
- class/record extractor patterns: `PairBox(left, right)`
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

Record destructuring:

```txt
let (value Int, label Str) = Amount(7, "world")
```

Class destructuring:

```txt
let (left Int, right Str) = Box(9, "boxed")
```

Skip pattern:

```txt
let (left Int, _, right Str) = (1, "drop", "keep")
```

Classes with private fields are not destructurable.

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
- `<-` for `for` iteration and `if` option binding
- `->` for function types and lambdas
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

Operator declarations use symbolic `def` forms on interfaces, classes, records, and enums:

```txt
def +(other Vec) Vec = Vec(this[0] + other[0], this[1] + other[1])
def -() Vec = Vec(-this[0], -this[1])
def [](index Int) Int = items[index]
def :+(value Int) Vec = ...
def ++(other Vec) Vec = ...
```

Current operator overloading constraints:

- Allowed to overload:
  - arithmetic: `+`, `-`, `*`, `/`, `%`
  - unary: unary `-`
  - collection-oriented: `[]`, `:+`, `:-`, `++`, `--`
  - symbolic custom forms with no built-in language meaning: `|`, `&`, `>>`, `<<`, `~`, `::`
- Not allowed to overload:
  - logical operators: `&&`, `||`, `!`
  - equality operators: `==`, `!=`
- Comparison operators are intended to work through `Ordering[T]` rather than custom operator declarations.
- Equality is intended to work through `Eq[T]` rather than custom operator declarations.

Newline continuation:

- Ordinary expressions are no longer broadly newline-insensitive.
- A newline continues the current expression only when the previous line clearly ends in a continuation form.
- Continuation tokens:
  - binary operators: `+`, `-`, `*`, `/`, `%`, `&&`, `||`, `==`, `!=`, `<`, `<=`, `>`, `>=`
  - symbolic/custom infix operators: `<<`, `>>`, `|`, `&`, `::`, `:+`, `:-`, `++`, `--`
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
- Body-introducing forms are intentionally looser:
  - `def ... =` may start its body on the next line
  - inline-body introducers such as `else` and `yield` may take a same-line body without braces
  - if that body moves to the next line, a `{ ... }` block is required
- So this is invalid:

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

if flag then return 1
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
- `hidden` on top-level `class` / `object` / `record` / `enum`
- `hidden` on fields
- `hidden` on methods

Top-level `def` and immutable bindings are private by default and only become importable across modules when marked `public`.

## Notes

This file is meant to describe the current surface syntax.

Ideas that are still under discussion belong in `features.md`, not here.
